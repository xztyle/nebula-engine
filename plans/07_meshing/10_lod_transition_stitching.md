# LOD Transition Stitching

## Problem

Nebula Engine uses level-of-detail (LOD) to reduce geometry for distant chunks. A nearby chunk might be at LOD 0 (full resolution, 32x32x32 voxels) while its neighbor is at LOD 1 (half resolution, effectively 16x16x16). When two adjacent chunks are at different LOD levels, their meshes have different vertex densities along the shared edge. The LOD 0 chunk has 32 vertices along the edge while the LOD 1 chunk has 16. This mismatch causes T-junctions: vertices on the high-LOD side sit in the middle of edges on the low-LOD side, creating hairline cracks where the background bleeds through. On a planetary scale with thousands of LOD transitions visible at any time, these cracks are visually unacceptable.

The stitching system must eliminate all cracks at LOD boundaries without introducing degenerate triangles, excessive geometry, or visible distortion.

## Solution

Implement LOD transition stitching in the `nebula_meshing` crate. When meshing a chunk, detect LOD mismatches with adjacent chunks and generate transition geometry that bridges the two resolutions.

### LOD Mismatch Detection

```rust
/// Information about the LOD relationship between a chunk and its 6 face neighbors.
pub struct LodContext {
    /// The LOD level of the central chunk being meshed.
    pub center_lod: u8,
    /// The LOD level of each face neighbor (if loaded).
    /// None means the neighbor is not loaded (treat as same LOD).
    pub neighbor_lods: [Option<u8>; 6],
}

impl LodContext {
    /// Check if a specific face has a LOD transition.
    pub fn has_transition(&self, direction: FaceDirection) -> bool {
        if let Some(neighbor_lod) = self.neighbor_lods[direction as usize] {
            neighbor_lod != self.center_lod
        } else {
            false
        }
    }

    /// Get the LOD difference for a face. Positive means neighbor is coarser.
    pub fn lod_difference(&self, direction: FaceDirection) -> i8 {
        if let Some(neighbor_lod) = self.neighbor_lods[direction as usize] {
            neighbor_lod as i8 - self.center_lod as i8
        } else {
            0
        }
    }
}
```

### Stitching Strategy: Edge Vertex Snapping

The higher-LOD (finer) chunk is responsible for stitching. Along the shared edge, any vertex on the high-LOD side that does not correspond to a vertex on the low-LOD grid is snapped to the nearest low-LOD edge position:

```rust
/// Snap high-LOD edge vertices to align with the low-LOD grid.
///
/// For a LOD 0 chunk adjacent to a LOD 1 chunk along the +X face:
/// - The +X edge of the LOD 0 mesh has vertices at x=32, y=0..32, z=0..32
/// - The LOD 1 chunk's -X edge has vertices at every 2nd position
/// - Odd-indexed edge vertices on the LOD 0 side are snapped to their
///   nearest even neighbor's position, collapsing them onto the low-LOD grid.
pub fn snap_edge_vertices(
    mesh: &mut ChunkMesh,
    direction: FaceDirection,
    lod_difference: u8,
    chunk_size: usize,
) {
    let step = 1 << lod_difference; // LOD 1 -> step 2, LOD 2 -> step 4

    for vertex in &mut mesh.vertices {
        if !is_on_face_boundary(vertex, direction, chunk_size) {
            continue;
        }

        // Snap the two tangential coordinates to the low-LOD grid
        let (u_idx, v_idx) = direction.tangential_axes();
        let u = vertex.position[u_idx] as usize;
        let v = vertex.position[v_idx] as usize;

        let snapped_u = (u / step) * step;
        let snapped_v = (v / step) * step;

        vertex.position[u_idx] = snapped_u as u8;
        vertex.position[v_idx] = snapped_v as u8;
    }
}
```

### Transition Strip Generation (Alternative)

For cases where vertex snapping produces degenerate triangles (zero-area), an alternative approach generates explicit transition strips along the boundary:

```rust
/// Generate a transition mesh strip along a LOD boundary.
/// The strip connects the high-LOD edge vertices to the low-LOD edge vertices
/// using a fan/strip of triangles that smoothly interpolate between the two grids.
pub fn generate_transition_strip(
    direction: FaceDirection,
    center_lod: u8,
    neighbor_lod: u8,
    chunk_size: usize,
) -> ChunkMesh {
    let mut strip = ChunkMesh::new();
    let high_step = 1usize;
    let low_step = 1 << (neighbor_lod - center_lod);

    // Walk along the boundary edge, generating triangles that bridge
    // the high-res edge to the low-res edge.
    // For each low-LOD segment, emit a triangle fan connecting the
    // low-LOD endpoints to all high-LOD vertices between them.

    let edge_size = chunk_size;
    let mut low_idx = 0;

    while low_idx < edge_size {
        let low_start = low_idx;
        let low_end = (low_idx + low_step).min(edge_size);

        // Emit triangles fanning from low_start to each high-LOD vertex
        for high_idx in (low_start + high_step)..low_end {
            strip.push_transition_triangle(
                direction,
                low_start,
                high_idx - high_step,
                high_idx,
            );
        }
        // Final triangle connecting to low_end
        strip.push_transition_triangle(
            direction,
            low_start,
            low_end - high_step,
            low_end,
        );

        low_idx = low_end;
    }

    strip
}
```

### Integration with Meshing Pipeline

LOD stitching runs after greedy meshing and before GPU upload:

1. The main thread provides `LodContext` alongside the `ChunkNeighborhood` when creating a `MeshingTask`.
2. After `greedy_mesh()` produces the base mesh, `apply_lod_stitching()` modifies edge vertices or appends transition geometry.
3. The final mesh (base + transitions) is returned as the `MeshingResult`.

### Watertightness Guarantee

The stitching produces a watertight boundary by ensuring every vertex on the shared edge exists in both meshes at the same position. The snapping approach guarantees this: after snapping, the high-LOD edge has the same vertex positions as the low-LOD edge (with some duplicate vertices that form degenerate triangles, which the GPU discards for free). The transition strip approach guarantees it by construction — the strip's outer edge matches the high-LOD mesh and its inner edge matches the low-LOD mesh.

## Outcome

The `nebula_meshing` crate exports `LodContext`, `snap_edge_vertices()`, and `generate_transition_strip()`. When adjacent chunks have different LOD levels, the meshing pipeline applies stitching to eliminate cracks at the boundary. The resulting mesh is watertight across LOD transitions. Running `cargo test -p nebula_meshing` passes all LOD stitching tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Where high-detail and low-detail chunks meet, stitching triangles fill the gaps. No cracks or T-junctions are visible at LOD boundaries.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_meshing` | workspace | `ChunkMesh`, `ChunkVertex`, `FaceDirection` from prior stories |
| `nebula_voxel` | workspace | Chunk size and LOD-level definitions |

No external crates required. LOD stitching is coordinate arithmetic on mesh vertices. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// When two adjacent chunks have the same LOD level, no stitching is needed.
    #[test]
    fn test_same_lod_neighbors_no_stitching() {
        let ctx = LodContext {
            center_lod: 0,
            neighbor_lods: [Some(0); 6],
        };

        for dir in FaceDirection::ALL {
            assert!(
                !ctx.has_transition(dir),
                "Same-LOD neighbors should not have a transition on {dir:?}"
            );
        }
    }

    /// LOD N next to LOD N+1 should produce transition geometry (or snapped vertices).
    #[test]
    fn test_lod_transition_produces_geometry() {
        let strip = generate_transition_strip(
            FaceDirection::PosX,
            0,  // center LOD
            1,  // neighbor LOD (coarser)
            32, // chunk size
        );

        assert!(
            !strip.is_empty(),
            "LOD 0 -> LOD 1 transition should produce transition geometry"
        );
        assert!(
            strip.triangle_count() > 0,
            "Transition strip should have triangles"
        );
    }

    /// After stitching, no cracks should exist: all edge vertices on the high-LOD
    /// side must align with the low-LOD grid positions.
    #[test]
    fn test_no_cracks_after_stitching() {
        let chunk_size = 32;
        let lod_diff = 1u8;
        let step = 1 << lod_diff; // 2

        // Create a mesh with vertices along the +X boundary at every integer position
        let mut mesh = ChunkMesh::new();
        for z in 0..=chunk_size as u8 {
            for y in 0..=chunk_size as u8 {
                mesh.vertices.push(ChunkVertex::new(
                    [chunk_size as u8, y, z],
                    FaceDirection::PosX,
                    0,
                    1,
                    [y, z],
                ));
            }
        }

        snap_edge_vertices(&mut mesh, FaceDirection::PosX, lod_diff, chunk_size);

        // After snapping, all edge vertex positions should be multiples of `step`
        for vertex in &mesh.vertices {
            let y = vertex.position[1] as usize;
            let z = vertex.position[2] as usize;
            assert_eq!(
                y % step, 0,
                "Y position {y} is not aligned to LOD grid (step={step})"
            );
            assert_eq!(
                z % step, 0,
                "Z position {z} is not aligned to LOD grid (step={step})"
            );
        }
    }

    /// Transition mesh should be watertight: every edge vertex on the transition
    /// strip matches either the high-LOD or low-LOD mesh edge.
    #[test]
    fn test_transition_mesh_is_watertight() {
        let strip = generate_transition_strip(
            FaceDirection::PosX,
            0,  // high LOD
            1,  // low LOD
            32,
        );

        // All indices should be valid
        let vertex_count = strip.vertices.len() as u32;
        for &idx in &strip.indices {
            assert!(
                idx < vertex_count,
                "Invalid index {idx} in transition strip (vertex count: {vertex_count})"
            );
        }

        // All triangles should have non-zero area (no fully degenerate triangles
        // in the explicit strip approach)
        for tri in strip.indices.chunks(3) {
            let v0 = strip.vertices[tri[0] as usize].position;
            let v1 = strip.vertices[tri[1] as usize].position;
            let v2 = strip.vertices[tri[2] as usize].position;
            let degenerate = v0 == v1 || v1 == v2 || v0 == v2;
            // Note: some degenerate triangles are acceptable in snapping mode
            // but not in strip generation mode
            assert!(
                !degenerate,
                "Transition strip should not contain degenerate triangles"
            );
        }
    }

    /// LodContext correctly reports transitions and differences.
    #[test]
    fn test_lod_context_reports_transitions() {
        let ctx = LodContext {
            center_lod: 0,
            neighbor_lods: [Some(0), Some(1), Some(0), Some(2), None, Some(0)],
        };

        assert!(!ctx.has_transition(FaceDirection::PosX)); // same LOD
        assert!(ctx.has_transition(FaceDirection::NegX));   // LOD 0 vs 1
        assert!(!ctx.has_transition(FaceDirection::PosY));  // same LOD
        assert!(ctx.has_transition(FaceDirection::NegY));   // LOD 0 vs 2
        assert!(!ctx.has_transition(FaceDirection::PosZ));  // None = no transition
        assert!(!ctx.has_transition(FaceDirection::NegZ));  // same LOD

        assert_eq!(ctx.lod_difference(FaceDirection::NegX), 1);
        assert_eq!(ctx.lod_difference(FaceDirection::NegY), 2);
    }
}
```
