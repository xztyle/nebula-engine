# Greedy Meshing

## Problem

After visible face detection, each exposed voxel face becomes a quad — two triangles. A flat 32x32 surface of the same material produces 1,024 individual quads (2,048 triangles) even though visually it is a single rectangle. This explosion of tiny triangles wastes GPU vertex processing time, inflates index buffers, and creates poor rasterizer utilization (most triangles are smaller than a pixel at distance). For a planet built from millions of chunks, naive per-face meshing would produce billions of triangles.

Greedy meshing solves this by merging coplanar, same-type adjacent faces into larger rectangular quads. A 32x32 flat surface of stone collapses to a single quad (2 triangles). In practice, greedy meshing reduces triangle counts by 10-50x depending on terrain complexity, making real-time rendering of large voxel worlds feasible.

## Solution

Implement a greedy meshing algorithm in the `nebula_meshing` crate. The algorithm processes each of the six face directions independently, sweeping layer by layer through the chunk and finding maximal rectangles of mergeable faces.

### Algorithm: Per-Direction Sweep

For each face direction (e.g., +Y, the "top" faces):

1. **Extract a 2D slice.** For a chunk of size `S`, iterate through `S` layers along the face's normal axis. Each layer is an `S x S` grid of face entries. A face entry is either `None` (no visible face at this position) or `Some(VoxelType)`.

2. **Greedy rectangle merging.** Iterate through the 2D grid. For each unvisited non-None cell:
   - Extend rightward as far as possible while the type matches and the cell is unvisited. This gives the width `w`.
   - Extend downward row by row, checking that the entire row of width `w` matches and is unvisited. This gives the height `h`.
   - Mark all cells in the `w x h` rectangle as visited.
   - Emit a quad with position, size `(w, h)`, face direction, and voxel type.

3. **Emit vertices and indices.** Each merged quad produces 4 vertices and 6 indices (two triangles). The vertices carry the quad's corner positions (relative to the chunk), the face normal, the voxel type (for texture lookup), and UV coordinates scaled to the quad's dimensions (for tiling textures).

```rust
pub fn greedy_mesh(
    chunk: &ChunkVoxelData,
    visible_faces: &[VisibleFaces],
    neighbors: &ChunkNeighborhood,
    registry: &VoxelTypeRegistry,
) -> ChunkMesh {
    let mut mesh = ChunkMesh::new();
    let size = chunk.size();

    for direction in FaceDirection::ALL {
        let (layer_axis, u_axis, v_axis) = direction.sweep_axes();
        let mut visited = vec![false; size * size];

        for layer in 0..size {
            // Reset visited mask for this layer
            visited.fill(false);

            for v in 0..size {
                for u in 0..size {
                    let (x, y, z) = axes_to_xyz(layer_axis, u_axis, v_axis, layer, u, v);
                    let idx = chunk.index(x, y, z);

                    if !visible_faces[idx].is_visible(direction) {
                        continue;
                    }
                    if visited[v * size + u] {
                        continue;
                    }

                    let voxel_type = chunk.get(x, y, z);

                    // Extend width
                    let mut w = 1;
                    while u + w < size {
                        let (nx, ny, nz) = axes_to_xyz(layer_axis, u_axis, v_axis, layer, u + w, v);
                        let ni = chunk.index(nx, ny, nz);
                        if !visible_faces[ni].is_visible(direction)
                            || visited[v * size + u + w]
                            || chunk.get(nx, ny, nz) != voxel_type
                        {
                            break;
                        }
                        w += 1;
                    }

                    // Extend height
                    let mut h = 1;
                    'outer: while v + h < size {
                        for du in 0..w {
                            let (nx, ny, nz) = axes_to_xyz(layer_axis, u_axis, v_axis, layer, u + du, v + h);
                            let ni = chunk.index(nx, ny, nz);
                            if !visible_faces[ni].is_visible(direction)
                                || visited[(v + h) * size + u + du]
                                || chunk.get(nx, ny, nz) != voxel_type
                            {
                                break 'outer;
                            }
                        }
                        h += 1;
                    }

                    // Mark visited
                    for dv in 0..h {
                        for du in 0..w {
                            visited[(v + dv) * size + u + du] = true;
                        }
                    }

                    // Emit quad
                    mesh.push_quad(direction, layer, u, v, w, h, voxel_type);
                }
            }
        }
    }

    mesh
}
```

### Quad Emission

`push_quad` computes the 4 corner vertices of the merged quad. Positions are in chunk-local coordinates (0..size for each axis). UVs are `(0,0)` to `(w,h)` so the texture tiles across the merged surface. The normal is the face direction's unit vector. Two triangles are emitted per quad with correct winding for front-face culling.

### Mergeable Criteria

Two adjacent faces merge only if:
- Same face direction (already guaranteed by the per-direction sweep).
- Same voxel type (texture/material must match).
- Both visible (not culled by adjacent solid blocks).
- Same AO values at shared vertices (optional — if AO breaks the merge, quads are split at AO boundaries; this is handled in the AO story but the merge predicate includes an AO check hook).

## Outcome

The `nebula_meshing` crate exports `greedy_mesh()` which takes chunk voxel data and the visibility bitmask from story 01, and returns a `ChunkMesh` of merged quads. A 32x32 flat surface collapses to 1 quad. A fully solid 32x32x32 chunk produces at most ~6 quads (one per exposed face of the cube). Running `cargo test -p nebula_meshing` passes all greedy meshing tests including the performance benchmark.

## Demo Integration

**Demo crate:** `nebula-demo`

Large flat areas are merged into single quads. A flat grass plain that was 1,024 individual quads becomes perhaps 12 large rectangles. The console logs `Quads: 12 (greedy) vs 1,024 (naive)`.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_voxel` | workspace | `ChunkVoxelData`, `VoxelType`, `VoxelTypeRegistry` |
| `nebula_meshing` | workspace | `ChunkMesh`, `VisibleFaces`, `FaceDirection` from story 01 |

No external crates required. The greedy meshing algorithm is pure arithmetic on arrays. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> VoxelTypeRegistry {
        let mut reg = VoxelTypeRegistry::new();
        reg.register(VoxelType::AIR, VoxelProperties { transparent: true, ..Default::default() });
        reg.register(VoxelType::STONE, VoxelProperties { transparent: false, ..Default::default() });
        reg.register(VoxelType::DIRT, VoxelProperties { transparent: false, ..Default::default() });
        reg
    }

    /// A flat 32x32 surface of one type on the +Y face should merge into exactly 1 quad.
    #[test]
    fn test_flat_surface_single_type_produces_one_quad() {
        let mut chunk = ChunkVoxelData::new_filled(32, VoxelType::AIR);
        // Fill the bottom layer (y=0) with stone
        for z in 0..32 {
            for x in 0..32 {
                chunk.set(x, 0, z, VoxelType::STONE);
            }
        }
        let neighbors = ChunkNeighborhood::all_air();
        let reg = registry();
        let visible = compute_visible_faces(&chunk, &neighbors, &reg);
        let mesh = greedy_mesh(&chunk, &visible, &neighbors, &reg);

        // Count quads for the +Y face direction only (top of the stone layer)
        let top_quads = mesh.count_quads_for_direction(FaceDirection::PosY);
        assert_eq!(top_quads, 1, "Flat 32x32 surface should merge to 1 quad");
    }

    /// A checkerboard pattern of stone and dirt on a surface should produce N separate quads
    /// (no merging possible across different types).
    #[test]
    fn test_checkerboard_produces_many_quads() {
        let mut chunk = ChunkVoxelData::new_filled(32, VoxelType::AIR);
        for z in 0..32 {
            for x in 0..32 {
                let vtype = if (x + z) % 2 == 0 { VoxelType::STONE } else { VoxelType::DIRT };
                chunk.set(x, 0, z, vtype);
            }
        }
        let neighbors = ChunkNeighborhood::all_air();
        let reg = registry();
        let visible = compute_visible_faces(&chunk, &neighbors, &reg);
        let mesh = greedy_mesh(&chunk, &visible, &neighbors, &reg);

        let top_quads = mesh.count_quads_for_direction(FaceDirection::PosY);
        // Each cell in the checkerboard is its own quad on the +Y face
        assert_eq!(top_quads, 32 * 32, "Checkerboard should produce 1024 quads on +Y");
    }

    /// An L-shaped surface should produce 2 or more quads (the greedy algorithm cannot
    /// merge a non-rectangular region into a single quad).
    #[test]
    fn test_l_shaped_surface_produces_multiple_quads() {
        let mut chunk = ChunkVoxelData::new_filled(32, VoxelType::AIR);
        // Horizontal bar: x=0..8, z=0
        for x in 0..8 {
            chunk.set(x, 0, 0, VoxelType::STONE);
        }
        // Vertical bar: x=0, z=1..8
        for z in 1..8 {
            chunk.set(0, 0, z, VoxelType::STONE);
        }
        let neighbors = ChunkNeighborhood::all_air();
        let reg = registry();
        let visible = compute_visible_faces(&chunk, &neighbors, &reg);
        let mesh = greedy_mesh(&chunk, &visible, &neighbors, &reg);

        let top_quads = mesh.count_quads_for_direction(FaceDirection::PosY);
        assert!(
            top_quads >= 2,
            "L-shaped surface should need at least 2 quads, got {top_quads}"
        );
    }

    /// Different voxel types should never be merged into the same quad.
    #[test]
    fn test_different_types_not_merged() {
        let mut chunk = ChunkVoxelData::new_filled(32, VoxelType::AIR);
        // Stone on left half, dirt on right half of one row
        for x in 0..16 {
            chunk.set(x, 0, 0, VoxelType::STONE);
        }
        for x in 16..32 {
            chunk.set(x, 0, 0, VoxelType::DIRT);
        }
        let neighbors = ChunkNeighborhood::all_air();
        let reg = registry();
        let visible = compute_visible_faces(&chunk, &neighbors, &reg);
        let mesh = greedy_mesh(&chunk, &visible, &neighbors, &reg);

        let top_quads = mesh.count_quads_for_direction(FaceDirection::PosY);
        assert!(
            top_quads >= 2,
            "Two different types must produce at least 2 quads, got {top_quads}"
        );
    }

    /// An empty chunk should produce an empty mesh with 0 quads and 0 vertices.
    #[test]
    fn test_empty_chunk_produces_zero_quads() {
        let chunk = ChunkVoxelData::new_filled(32, VoxelType::AIR);
        let neighbors = ChunkNeighborhood::all_air();
        let reg = registry();
        let visible = compute_visible_faces(&chunk, &neighbors, &reg);
        let mesh = greedy_mesh(&chunk, &visible, &neighbors, &reg);

        assert_eq!(mesh.vertices.len(), 0);
        assert_eq!(mesh.indices.len(), 0);
    }

    /// Performance: meshing a full 32x32x32 chunk should complete in under 1ms.
    #[test]
    fn test_meshing_performance_32_cubed() {
        let chunk = ChunkVoxelData::new_filled(32, VoxelType::STONE);
        let neighbors = ChunkNeighborhood::all_air();
        let reg = registry();
        let visible = compute_visible_faces(&chunk, &neighbors, &reg);

        let start = std::time::Instant::now();
        let _mesh = greedy_mesh(&chunk, &visible, &neighbors, &reg);
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 1,
            "Greedy meshing took {}ms, expected <1ms",
            elapsed.as_millis()
        );
    }
}
```
