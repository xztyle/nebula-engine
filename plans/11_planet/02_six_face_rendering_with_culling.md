# Six-Face Rendering with Culling

## Problem

Story 01 renders a single cube face, which proves the cubesphere-voxel pipeline works end-to-end. But a planet has six faces, and a real camera orbits freely -- it does not stare down at one face from a fixed position. When all six faces are active, the engine must decide which faces and which chunks within those faces are worth loading and rendering. Without culling, the engine loads and draws geometry for faces the camera cannot see (e.g., the underside of the planet), wasting both memory and GPU time. Each face also has a quadtree (from the cubesphere subdivision system in Epic 05) that determines which chunks to load at each LOD level. Crucially, chunks at face boundaries must mesh seamlessly: a player walking across the edge between the PosX and PosZ faces must see continuous terrain with no gaps, cracks, or T-junctions. This story scales the single-face proof-of-concept to a full planet with all six faces active simultaneously.

## Solution

### Per-Face Quadtree Management

Each cube face maintains its own quadtree that subdivides the face into chunks based on camera distance. The quadtree root covers the entire face. Children are created when the camera is close enough to warrant higher detail, and pruned when the camera moves away:

```rust
use nebula_cubesphere::{CubeFace, ChunkAddress};
use nebula_lod::Quadtree;

pub struct PlanetFaces {
    pub face_trees: [FaceState; 6],
    pub planet_radius: f64,
}

pub struct FaceState {
    pub face: CubeFace,
    pub quadtree: Quadtree,
    pub visible: bool,
}

impl PlanetFaces {
    pub fn new(planet_radius: f64) -> Self {
        Self {
            face_trees: CubeFace::ALL.map(|face| FaceState {
                face,
                quadtree: Quadtree::new_for_face(face),
                visible: true,
            }),
            planet_radius,
        }
    }

    /// Update all six quadtrees based on camera position.
    /// Returns the set of chunk addresses to load/unload.
    pub fn update(&mut self, camera_pos: &WorldPosition) -> QuadtreeUpdateResult {
        let mut result = QuadtreeUpdateResult::default();
        for face_state in &mut self.face_trees {
            let face_update = face_state.quadtree.update(
                camera_pos,
                self.planet_radius,
                face_state.face,
            );
            result.merge(face_update);
        }
        result
    }
}
```

### Face-Level Frustum Culling

Before processing a face's quadtree, perform a coarse test: compute the bounding sphere of the face (a hemisphere cap) and test it against the view frustum. If the entire face is outside the frustum, mark it as invisible and skip all chunk processing for that face:

```rust
use nebula_coords::Frustum128;

impl PlanetFaces {
    /// Mark faces as visible or invisible based on frustum culling.
    /// Returns the number of faces marked visible.
    pub fn cull_faces(&mut self, frustum: &Frustum128) -> u32 {
        let mut visible_count = 0;
        for face_state in &mut self.face_trees {
            let face_aabb = compute_face_aabb(
                face_state.face,
                self.planet_radius,
            );
            face_state.visible = frustum.contains_aabb(&face_aabb)
                != nebula_coords::Intersection::Outside;
            if face_state.visible {
                visible_count += 1;
            }
        }
        visible_count
    }
}

/// Compute an AABB128 that encloses the hemisphere cap of this cube face.
fn compute_face_aabb(face: CubeFace, planet_radius: f64) -> AABB128 {
    let normal = face.normal();
    let r = planet_radius as i128;
    let center = WorldPosition {
        x: (normal.x * planet_radius) as i128,
        y: (normal.y * planet_radius) as i128,
        z: (normal.z * planet_radius) as i128,
    };
    // The bounding box extends ±radius perpendicular to the face normal
    // and from 0 to +radius along the normal.
    AABB128 {
        min: WorldPosition {
            x: center.x - r,
            y: center.y - r,
            z: center.z - r,
        },
        max: WorldPosition {
            x: center.x + r,
            y: center.y + r,
            z: center.z + r,
        },
    }
}
```

### Chunk-Level Frustum Culling

Within each visible face, individual chunks are also culled. This uses the local-space f32 frustum (converted from the camera-relative view), testing each chunk's bounding volume against the frustum before meshing or drawing:

```rust
use glam::Vec3;

pub struct ChunkBounds {
    pub center: Vec3,
    pub half_extents: Vec3,
}

/// Cull individual chunks within a visible face.
/// Returns only the chunk addresses that are within the frustum.
pub fn cull_chunks(
    chunks: &[ChunkAddress],
    chunk_bounds: &HashMap<ChunkAddress, ChunkBounds>,
    frustum: &LocalFrustum,
) -> Vec<ChunkAddress> {
    chunks
        .iter()
        .filter(|addr| {
            if let Some(bounds) = chunk_bounds.get(addr) {
                frustum.intersects_aabb(bounds.center, bounds.half_extents)
            } else {
                true // If bounds unknown, conservatively include
            }
        })
        .copied()
        .collect()
}
```

### Seamless Face Boundary Meshing

Chunks at the edge of one face share voxel data with the adjacent face. When meshing an edge chunk, the mesher must read neighbor voxels from the adjacent face's chunk to determine which faces are visible at the boundary. Without this, the mesher would treat edge voxels as having air neighbors, creating false faces (visible cracks where two faces meet):

```rust
use nebula_cubesphere::face_neighbor;

/// Resolve the neighbor chunk address across a face boundary.
/// If the neighbor coordinate goes beyond [0, face_size), wrap to the adjacent face.
pub fn resolve_cross_face_neighbor(
    addr: &ChunkAddress,
    direction: Direction2D,
    face_size: i32,
) -> ChunkAddress {
    let (nu, nv) = match direction {
        Direction2D::PosU => (addr.u + 1, addr.v),
        Direction2D::NegU => (addr.u - 1, addr.v),
        Direction2D::PosV => (addr.u, addr.v + 1),
        Direction2D::NegV => (addr.u, addr.v - 1),
    };

    if nu >= 0 && nu < face_size && nv >= 0 && nv < face_size {
        // Still on the same face.
        ChunkAddress { face: addr.face, u: nu, v: nv, lod: addr.lod }
    } else {
        // Crossed a face boundary -- look up the adjacent face and remap coordinates.
        let (adj_face, remapped_u, remapped_v) =
            face_neighbor(addr.face, direction, nu, nv, face_size);
        ChunkAddress { face: adj_face, u: remapped_u, v: remapped_v, lod: addr.lod }
    }
}
```

### Render Loop

The per-frame render loop becomes:

1. Update the `Frustum128` from the camera's world position and orientation.
2. Call `cull_faces()` to mark invisible faces.
3. For each visible face, update the quadtree and collect chunk addresses.
4. Cull individual chunks against the local f32 frustum.
5. Load/unload chunks as needed (terrain gen + meshing).
6. Submit draw calls for all visible chunk meshes.

```rust
pub fn render_planet(
    planet: &mut PlanetFaces,
    frustum_128: &Frustum128,
    frustum_local: &LocalFrustum,
    pipeline: &UnlitPipeline,
    ctx: &RenderContext,
) {
    planet.cull_faces(frustum_128);

    let mut all_visible_chunks = Vec::new();
    for face_state in &planet.face_trees {
        if !face_state.visible {
            continue;
        }
        let face_chunks = face_state.quadtree.leaf_addresses();
        let visible = cull_chunks(&face_chunks, &face_state.chunk_bounds, frustum_local);
        all_visible_chunks.extend(visible);
    }

    // Load, mesh, displace, and draw all visible chunks.
    submit_draw_calls(ctx, pipeline, &all_visible_chunks);
}
```

## Outcome

All six cube faces render simultaneously. The quadtree on each face manages chunk subdivision based on camera distance. Face-level frustum culling skips entire faces that are behind the camera or off-screen. Chunk-level culling further reduces draw calls within visible faces. Face boundaries are seamless with no visible gaps between adjacent faces. The planet is renderable from any camera angle, not just the fixed top-down view of story 01.

## Demo Integration

**Demo crate:** `nebula-demo`

The planet is now fully renderable from any angle. All six cube faces are visible when orbiting. Face boundaries are seamless with no visible gaps.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | GPU rendering |
| `glam` | `0.29` | Frustum math, vector operations |
| `bytemuck` | `1.21` | Buffer serialization |

Internal dependencies: `nebula-cubesphere`, `nebula-voxel`, `nebula-mesh`, `nebula-terrain`, `nebula-render`, `nebula-coords`, `nebula-lod`. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_cubesphere::CubeFace;

    #[test]
    fn test_all_six_faces_have_quadtrees() {
        let planet = PlanetFaces::new(6_371_000.0);
        assert_eq!(planet.face_trees.len(), 6);
        for (i, face_state) in planet.face_trees.iter().enumerate() {
            assert_eq!(
                face_state.face,
                CubeFace::ALL[i],
                "Face {i} should be {:?}",
                CubeFace::ALL[i]
            );
            assert!(
                face_state.quadtree.root().is_some(),
                "Quadtree for {:?} should have a root node",
                face_state.face
            );
        }
    }

    #[test]
    fn test_only_visible_faces_are_processed() {
        let mut planet = PlanetFaces::new(6_371_000.0);

        // Camera on the +Y axis looking down: only PosY and possibly
        // adjacent faces should be visible; NegY should be culled.
        let frustum = build_test_frustum_looking_at(
            WorldPosition { x: 0, y: 7_000_000_000, z: 0 }, // 7000 km up on Y
            WorldPosition { x: 0, y: 0, z: 0 },             // looking at origin
        );
        let visible = planet.cull_faces(&frustum);

        // At least PosY must be visible.
        let pos_y = &planet.face_trees[CubeFace::PosY.index()];
        assert!(pos_y.visible, "PosY face should be visible when camera is above it");

        // NegY (bottom face, opposite side of planet) must be culled.
        let neg_y = &planet.face_trees[CubeFace::NegY.index()];
        assert!(!neg_y.visible, "NegY face should be culled when camera is above PosY");

        // Total visible faces should be less than 6 (at least the back face is culled).
        assert!(
            visible < 6,
            "Expected fewer than 6 visible faces, got {visible}"
        );
    }

    #[test]
    fn test_face_boundaries_seamless() {
        // Load chunks along the PosX/PosZ boundary and verify no gap.
        let planet_radius = 1000.0;
        let face_size = 16; // 16 chunks per face edge

        // Get the edge chunk on PosX (u = face_size - 1) and its neighbor on PosZ.
        let edge_addr = ChunkAddress {
            face: CubeFace::PosX,
            u: face_size - 1,
            v: 4,
            lod: 0,
        };
        let neighbor = resolve_cross_face_neighbor(
            &edge_addr,
            Direction2D::PosU,
            face_size,
        );

        // The neighbor should be on a different face.
        assert_ne!(
            neighbor.face, edge_addr.face,
            "Cross-face neighbor should be on a different face"
        );

        // Generate terrain for both chunks and verify the edge voxels match.
        let gen = TerrainGenerator::with_seed(42);
        let chunk_a = gen.generate_chunk(&edge_addr);
        let chunk_b = gen.generate_chunk(&neighbor);

        // The shared edge voxels should produce continuous terrain.
        // Check that the height at the boundary is within 1 voxel of each other.
        for v in 0..32 {
            let height_a = chunk_a.surface_height_at_edge(Direction2D::PosU, v);
            let height_b = chunk_b.surface_height_at_edge(Direction2D::NegU, v);
            assert!(
                (height_a as i32 - height_b as i32).abs() <= 1,
                "Boundary height mismatch at v={v}: {height_a} vs {height_b}"
            );
        }
    }

    #[test]
    fn test_total_chunk_count_is_reasonable() {
        let mut planet = PlanetFaces::new(6_371_000.0);
        let camera_pos = WorldPosition { x: 0, y: 6_381_000_000, z: 0 }; // 10km above surface
        let result = planet.update(&camera_pos);

        let total_chunks: usize = planet
            .face_trees
            .iter()
            .map(|fs| fs.quadtree.leaf_count())
            .sum();

        // With reasonable LOD settings, we expect between 100 and 10,000 chunks.
        assert!(
            total_chunks >= 100 && total_chunks <= 10_000,
            "Total chunk count {total_chunks} is outside reasonable range [100, 10000]"
        );
    }

    #[test]
    fn test_render_time_is_bounded() {
        let (ctx, pipeline) = create_test_render_context();
        let mut planet = PlanetFaces::new(1_000.0); // Small planet for test speed
        let frustum = build_test_frustum_default();

        let start = std::time::Instant::now();
        planet.cull_faces(&frustum);
        for face_state in &planet.face_trees {
            if face_state.visible {
                let _ = face_state.quadtree.leaf_addresses();
            }
        }
        let elapsed = start.elapsed();

        // Culling and quadtree traversal should take less than 10ms.
        assert!(
            elapsed.as_millis() < 10,
            "Face culling + quadtree traversal took {}ms, expected < 10ms",
            elapsed.as_millis()
        );
    }
}
```
