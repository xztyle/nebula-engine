# LOD Transition Seams

## Problem

When two adjacent chunks have different LOD levels, their meshes have mismatched vertex densities at the shared boundary. A LOD 0 chunk (32x32x32) has 32 vertices along an edge, while a LOD 1 neighbor (16x16x16) has only 16 vertices along the same edge. The higher-resolution mesh's boundary vertices do not align with the lower-resolution mesh's boundary vertices, producing visible cracks (T-junctions) where light leaks through, z-fighting artifacts, and jagged seams that destroy the illusion of continuous terrain. This problem is inherent to any multi-resolution voxel system and must be solved at the meshing stage. On a cubesphere planet this is especially pronounced because chunks curve along the sphere surface, making simple planar stitching insufficient.

## Solution

Implement LOD transition seam elimination in the `nebula_meshing` crate using two complementary techniques: **edge vertex constraining** (snapping higher-LOD boundary vertices to match the lower-LOD neighbor's edge) and **skirt geometry** (adding vertical face strips along chunk boundaries to hide any remaining gaps).

### Edge Vertex Constraining

When meshing a chunk that has a neighbor at a coarser LOD level, the mesher constrains the higher-resolution chunk's boundary vertices to align with the lower-resolution neighbor.

```rust
/// Describes the LOD relationship between a chunk and its neighbor on one edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NeighborLodRelation {
    /// Same LOD level — no constraining needed.
    Same,
    /// This chunk has higher resolution than the neighbor.
    /// The edge vertices of this chunk must be constrained downward.
    HigherThanNeighbor {
        /// The LOD difference (always positive). Typically 1.
        lod_diff: u8,
    },
    /// This chunk has lower resolution than the neighbor.
    /// The neighbor will constrain its vertices; this chunk does nothing special.
    LowerThanNeighbor,
}

/// Describes the LOD context for meshing a chunk, including neighbor relationships.
pub struct ChunkLodContext {
    /// LOD level of this chunk.
    pub lod: u8,
    /// Neighbor LOD relationship for each of the 4 horizontal edges (+X, -X, +Z, -Z)
    /// and 2 vertical edges (+Y, -Y).
    pub neighbors: [NeighborLodRelation; 6],
}
```

The constraining algorithm works as follows:

```rust
/// Constrain edge vertices of a high-res chunk to match a low-res neighbor.
///
/// For each boundary face that lies on the edge adjacent to a coarser neighbor:
/// 1. Identify which boundary vertices correspond to the lower-res grid.
///    Every 2^lod_diff vertices in the high-res grid maps to one vertex in the low-res grid.
/// 2. Snap intermediate high-res boundary vertices to the interpolated position
///    between the two nearest low-res-aligned vertices.
/// 3. This eliminates T-junctions by ensuring the high-res edge exactly matches
///    the low-res edge's tessellation.
pub fn constrain_edge_vertices(
    mesh: &mut ChunkMesh,
    edge: FaceDirection,
    lod_diff: u8,
) {
    let step = 1 << lod_diff; // e.g., 2 for lod_diff=1

    for vertex in mesh.boundary_vertices_on_edge(edge) {
        let edge_coord = vertex.position_along_edge();
        let lower = (edge_coord / step) * step;
        let upper = lower + step;

        if edge_coord % step != 0 {
            // This vertex is between two low-res grid points — interpolate
            let t = (edge_coord - lower) as f32 / step as f32;
            let pos_lower = mesh.vertex_at_edge_coord(edge, lower);
            let pos_upper = mesh.vertex_at_edge_coord(edge, upper);
            vertex.set_position(pos_lower.lerp(pos_upper, t));
        }
    }
}
```

### Skirt Geometry

Even after vertex constraining, floating-point precision and the curvature of the cubesphere can leave sub-pixel gaps. Skirt geometry provides a safety net: short vertical face strips are added along chunk boundaries, extending slightly below (or inward toward the planet center) the surface. These skirts are invisible under normal viewing angles but cover any remaining hairline cracks.

```rust
/// Generate skirt geometry along a chunk boundary edge.
/// The skirt extends `skirt_depth` units below the surface along the planet's
/// radial direction.
pub fn generate_skirt(
    mesh: &mut ChunkMesh,
    edge: FaceDirection,
    skirt_depth: f32,
    planet_center: &Vec3,
) {
    let boundary_verts = mesh.ordered_boundary_vertices(edge);

    for window in boundary_verts.windows(2) {
        let v0 = window[0];
        let v1 = window[1];

        // Compute the inward (toward planet center) direction for each vertex
        let inward_0 = (planet_center - v0.position).normalize() * skirt_depth;
        let inward_1 = (planet_center - v1.position).normalize() * skirt_depth;

        let v0_low = v0.position + inward_0;
        let v1_low = v1.position + inward_1;

        // Emit a quad (two triangles) forming the skirt face
        mesh.push_triangle(v0.position, v1.position, v0_low);
        mesh.push_triangle(v1.position, v1_low, v0_low);
    }
}
```

### Integration with Meshing Pipeline

The seam-fixing pass is integrated into the chunk meshing pipeline after greedy meshing but before GPU upload:

1. Greedy mesh generates the chunk's geometry as usual.
2. `constrain_edge_vertices()` is called for each edge that has a coarser neighbor.
3. `generate_skirt()` is called for each edge that has any LOD difference.
4. The final mesh (with constrained vertices and skirts) is uploaded to the GPU.

## Outcome

The `nebula_meshing` crate exports `ChunkLodContext`, `NeighborLodRelation`, `constrain_edge_vertices()`, and `generate_skirt()`. When meshing a chunk, the LOD context from the quadtree is passed in, and the resulting mesh has no visible seams or T-junctions at LOD boundaries. Running `cargo test -p nebula_meshing` passes all seam-related tests.

## Demo Integration

**Demo crate:** `nebula-demo`

LOD boundaries are seamless. No cracks or T-junctions are visible where high-detail and low-detail chunks meet. Skirt geometry fills any remaining gaps.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_math` | workspace | `Vec3`, `lerp`, vector math |
| `nebula_voxel` | workspace | Chunk data types |
| `nebula_lod` | workspace | `LodThresholds`, LOD level types |
| `glam` | `0.29` | Fast SIMD vector math for vertex manipulation |

The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Two adjacent chunks at the same LOD level should have no seam artifacts.
    /// Their boundary vertices should already align without any constraining.
    #[test]
    fn test_same_lod_neighbors_no_seam() {
        let chunk_a = generate_test_mesh(/*lod=*/ 0);
        let chunk_b = generate_test_mesh(/*lod=*/ 0);

        let boundary_a = chunk_a.boundary_vertices(FaceDirection::PosX);
        let boundary_b = chunk_b.boundary_vertices(FaceDirection::NegX);

        // Every vertex on chunk A's +X edge should have a matching vertex
        // on chunk B's -X edge at the same position.
        for va in &boundary_a {
            let matched = boundary_b.iter().any(|vb| {
                (va.position - vb.position).length() < f32::EPSILON
            });
            assert!(matched, "vertex {:?} has no match on neighbor", va.position);
        }
    }

    /// A LOD 0 chunk next to a LOD 1 chunk should have its boundary vertices
    /// constrained so there are no T-junctions.
    #[test]
    fn test_lod_difference_of_1_handled() {
        let mut mesh = generate_test_mesh(/*lod=*/ 0);
        let context = ChunkLodContext {
            lod: 0,
            neighbors: neighbor_context_with(FaceDirection::PosX, NeighborLodRelation::HigherThanNeighbor { lod_diff: 1 }),
        };

        constrain_edge_vertices(&mut mesh, FaceDirection::PosX, 1);

        // After constraining, boundary vertices at odd indices should be
        // interpolated between their even neighbors.
        let boundary = mesh.ordered_boundary_vertices(FaceDirection::PosX);
        for i in (1..boundary.len()).step_by(2) {
            let expected = boundary[i - 1].position.lerp(boundary[i + 1].position, 0.5);
            let actual = boundary[i].position;
            assert!(
                (actual - expected).length() < 1e-5,
                "vertex {i} should be interpolated: expected {expected:?}, got {actual:?}"
            );
        }
    }

    /// Skirt geometry should cover the gap between chunk boundaries.
    #[test]
    fn test_skirt_geometry_covers_gaps() {
        let mut mesh = generate_test_mesh(/*lod=*/ 0);
        let vert_count_before = mesh.vertex_count();

        generate_skirt(
            &mut mesh,
            FaceDirection::PosX,
            /*skirt_depth=*/ 0.5,
            &Vec3::ZERO, // planet center
        );

        // Skirt should add vertices
        assert!(mesh.vertex_count() > vert_count_before);

        // Skirt vertices should extend inward from the boundary
        let skirt_verts = &mesh.vertices()[vert_count_before..];
        for v in skirt_verts {
            // Skirt vertices should be closer to planet center than boundary
            assert!(v.position.length() < mesh.vertices()[0].position.length() + 0.01);
        }
    }

    /// The final mesh should have no T-junctions anywhere.
    #[test]
    fn test_no_t_junctions_in_final_mesh() {
        let mut mesh = generate_test_mesh(/*lod=*/ 0);
        constrain_edge_vertices(&mut mesh, FaceDirection::PosX, 1);

        // Verify: no edge in the mesh has a vertex from another triangle
        // lying on it (the definition of a T-junction).
        assert!(
            !mesh.has_t_junctions(),
            "mesh should not contain T-junctions after constraining"
        );
    }

    /// Visual continuity: the surface normals along the boundary should be
    /// smooth (no abrupt normal flips that would cause visible shading seams).
    #[test]
    fn test_visual_continuity_across_boundary() {
        let mut mesh = generate_test_mesh(/*lod=*/ 0);
        constrain_edge_vertices(&mut mesh, FaceDirection::PosX, 1);

        let boundary = mesh.ordered_boundary_vertices(FaceDirection::PosX);
        for window in boundary.windows(2) {
            let dot = window[0].normal.dot(window[1].normal);
            assert!(
                dot > 0.9,
                "adjacent boundary normals should be smooth, got dot product {dot}"
            );
        }
    }
}
```
