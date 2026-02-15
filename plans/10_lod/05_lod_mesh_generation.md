# LOD Mesh Generation

## Problem

The engine's greedy meshing algorithm (from the `nebula_meshing` crate) was originally designed to operate on fixed 32x32x32 chunk data. With the introduction of LOD-dependent chunk resolutions (32x32x32 at LOD 0, 16x16x16 at LOD 1, 8x8x8 at LOD 2, etc.), the mesher must handle variable-resolution input data. The same greedy meshing algorithm applies — it merges adjacent visible faces of the same type into larger quads — but it operates on a grid with fewer cells at higher LOD levels. Each voxel at a higher LOD level represents a larger physical volume (2^N meters per side at LOD N), so the output mesh quads are correspondingly larger in world space while the vertex count is dramatically lower. The mesher must produce valid, efficient geometry at every LOD level and scale vertex positions correctly so that the mesh covers the same world-space area regardless of LOD.

## Solution

Extend the greedy meshing pipeline in `nebula_meshing` to accept `LodChunkData` (variable-resolution chunks) and produce meshes with correctly scaled vertex positions.

### LOD-Aware Meshing Entry Point

```rust
/// Generate a mesh from an LOD chunk, producing geometry with correct world-space
/// vertex positions.
pub fn mesh_lod_chunk(
    chunk: &LodChunkData,
    neighbors: &ChunkNeighborhood,
    registry: &VoxelTypeRegistry,
    lod_context: &ChunkLodContext,
) -> ChunkMesh {
    let resolution = chunk.resolution();
    let voxel_scale = chunk.voxel_world_size(); // 2^lod meters per voxel

    // Step 1: Compute visible faces at the chunk's native resolution
    let visible = compute_visible_faces_lod(chunk, neighbors, registry);

    // Step 2: Run greedy meshing on the reduced-resolution grid
    let mut mesh = greedy_mesh(chunk, &visible, resolution);

    // Step 3: Scale vertex positions from grid space to world space
    mesh.scale_vertices(voxel_scale);

    // Step 4: Apply LOD transition seam fixes (from story 04)
    apply_lod_seam_fixes(&mut mesh, lod_context);

    mesh
}
```

### Greedy Meshing at Variable Resolution

The greedy meshing algorithm is parametric on grid resolution. Instead of hardcoding 32 as the grid size, it uses the chunk's resolution:

```rust
fn greedy_mesh(
    chunk: &LodChunkData,
    visible: &[VisibleFaces],
    resolution: u32,
) -> ChunkMesh {
    let mut mesh = ChunkMesh::new();

    // For each of the 6 face directions, sweep slices through the grid
    for dir in FaceDirection::ALL {
        let (u_axis, v_axis, w_axis) = dir.axes();

        for w in 0..resolution {
            // Build a 2D mask of visible faces on this slice
            let mut mask = vec![FaceInfo::NONE; (resolution * resolution) as usize];
            for v in 0..resolution {
                for u in 0..resolution {
                    let idx = chunk.index_along_axes(u, v, w, u_axis, v_axis, w_axis);
                    if visible[idx].is_visible(dir) {
                        let voxel = chunk.get_by_index(idx);
                        mask[(u + v * resolution) as usize] = FaceInfo::from_voxel(voxel);
                    }
                }
            }

            // Greedy merge: find maximal rectangles of identical FaceInfo
            greedy_merge_slice(&mask, resolution, |quad| {
                mesh.emit_quad(dir, w, quad);
            });
        }
    }

    mesh
}
```

### Vertex Scaling

After meshing in grid-local coordinates (where each voxel is 1 unit), vertices are scaled to world space:

```rust
impl ChunkMesh {
    /// Scale all vertex positions from grid coordinates to world coordinates.
    /// At LOD N, each grid unit corresponds to 2^N meters.
    pub fn scale_vertices(&mut self, voxel_scale: f32) {
        for vertex in &mut self.vertices {
            vertex.position *= voxel_scale;
        }
    }
}
```

### Triangle Count Scaling

At LOD N, the grid resolution is `32 / 2^N` along each axis. For a 2D slice, the greedy mesher operates on a `(32/2^N)^2` grid. Since the number of potential faces scales with the surface area of the grid, the triangle count scales approximately as `1 / 4^N` relative to LOD 0:

| LOD | Grid | Max Faces per Slice | Approx. Triangle Ratio |
|-----|------|--------------------|-----------------------|
| 0 | 32x32 | 1,024 | 1.0 |
| 1 | 16x16 | 256 | 1/4 |
| 2 | 8x8 | 64 | 1/16 |
| 3 | 4x4 | 16 | 1/64 |
| 4 | 2x2 | 4 | 1/256 |

In practice, greedy merging further reduces triangle counts, but the scaling relationship holds as an upper bound.

### Meshing Performance

Because the greedy meshing algorithm is O(resolution^3) and resolution halves with each LOD level, meshing time decreases by 8x per LOD level:

- LOD 0: O(32^3) = O(32,768) iterations
- LOD 1: O(16^3) = O(4,096) iterations
- LOD 2: O(8^3) = O(512) iterations
- LOD 3: O(4^3) = O(64) iterations

This means the engine can mesh many more distant chunks per frame than nearby chunks, which aligns perfectly with the fact that there are far more distant chunks visible.

## Outcome

The `nebula_meshing` crate exports `mesh_lod_chunk()` and the updated greedy meshing pipeline that accepts variable-resolution chunk data. The mesher produces correctly scaled geometry at every LOD level, with triangle counts decreasing proportionally. Running `cargo test -p nebula_meshing` passes all LOD meshing tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Each LOD level produces proportionally fewer triangles. The console logs `LOD 0: 2048 tris, LOD 1: 512 tris, LOD 2: 128 tris`.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_voxel` | workspace | `LodChunkData`, `VoxelTypeRegistry` |
| `nebula_lod` | workspace | `ChunkLodContext`, LOD level types |
| `nebula_math` | workspace | Vector types, coordinate math |
| `glam` | `0.29` | SIMD-accelerated vertex math |

The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_solid_lod_chunk(lod: u8) -> LodChunkData {
        let mut chunk = LodChunkData::new(lod);
        let res = chunk.resolution();
        for z in 0..res {
            for y in 0..res / 2 { // bottom half is solid
                for x in 0..res {
                    chunk.set(x, y, z, VoxelTypeId::STONE);
                }
            }
        }
        chunk
    }

    /// LOD 0 mesh should have the most triangles of any LOD level.
    #[test]
    fn test_lod_0_mesh_has_most_triangles() {
        let mesh_0 = mesh_lod_chunk_simple(0);
        let mesh_1 = mesh_lod_chunk_simple(1);
        let mesh_2 = mesh_lod_chunk_simple(2);

        assert!(
            mesh_0.triangle_count() > mesh_1.triangle_count(),
            "LOD 0 ({}) should have more triangles than LOD 1 ({})",
            mesh_0.triangle_count(),
            mesh_1.triangle_count()
        );
        assert!(
            mesh_1.triangle_count() > mesh_2.triangle_count(),
            "LOD 1 ({}) should have more triangles than LOD 2 ({})",
            mesh_1.triangle_count(),
            mesh_2.triangle_count()
        );
    }

    /// LOD N mesh should have approximately 1/4^N the triangles of LOD 0
    /// (within a reasonable factor due to greedy merging differences).
    #[test]
    fn test_lod_n_triangle_ratio() {
        let mesh_0 = mesh_lod_chunk_simple(0);
        let mesh_2 = mesh_lod_chunk_simple(2);

        let ratio = mesh_0.triangle_count() as f64 / mesh_2.triangle_count() as f64;
        // At LOD 2, we expect ~16x fewer triangles (1/4^2 = 1/16).
        // Allow a factor of 2 tolerance for greedy merging effects.
        assert!(
            ratio > 8.0 && ratio < 32.0,
            "expected ~16x ratio, got {ratio:.1}x"
        );
    }

    /// Mesh should be valid (no degenerate triangles, proper winding) at every LOD level.
    #[test]
    fn test_mesh_valid_at_all_lod_levels() {
        for lod in 0..=4 {
            let mesh = mesh_lod_chunk_simple(lod);
            assert!(
                mesh.triangle_count() > 0,
                "LOD {lod} mesh should have some triangles"
            );
            assert!(
                !mesh.has_degenerate_triangles(),
                "LOD {lod} mesh has degenerate triangles"
            );
            assert!(
                mesh.has_consistent_winding(),
                "LOD {lod} mesh has inconsistent winding order"
            );
        }
    }

    /// Meshing time should decrease with higher LOD levels.
    #[test]
    fn test_meshing_time_decreases_with_lod() {
        let start_0 = std::time::Instant::now();
        let _mesh_0 = mesh_lod_chunk_simple(0);
        let time_0 = start_0.elapsed();

        let start_2 = std::time::Instant::now();
        let _mesh_2 = mesh_lod_chunk_simple(2);
        let time_2 = start_2.elapsed();

        assert!(
            time_0 > time_2,
            "LOD 0 meshing ({time_0:?}) should take longer than LOD 2 ({time_2:?})"
        );
    }

    /// The mesh should correctly represent the low-res voxel data:
    /// a solid half-chunk should have faces on the top surface and sides.
    #[test]
    fn test_mesh_represents_low_res_data() {
        for lod in 0..=3 {
            let chunk = make_solid_lod_chunk(lod);
            let mesh = mesh_lod_chunk(
                &chunk,
                &ChunkNeighborhood::all_air(),
                &default_registry(),
                &ChunkLodContext::no_neighbors(lod),
            );

            // Should have top face, bottom face, and 4 side faces
            assert!(
                mesh.triangle_count() >= 6 * 2, // at least 6 quads = 12 triangles
                "LOD {lod} mesh should represent all visible faces of the half-solid chunk, got {} triangles",
                mesh.triangle_count()
            );
        }
    }

    fn mesh_lod_chunk_simple(lod: u8) -> ChunkMesh {
        let chunk = make_solid_lod_chunk(lod);
        mesh_lod_chunk(
            &chunk,
            &ChunkNeighborhood::all_air(),
            &default_registry(),
            &ChunkLodContext::no_neighbors(lod),
        )
    }
}
```
