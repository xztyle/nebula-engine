# Visible Face Detection

## Problem

Every solid voxel in a chunk has six faces, but the vast majority of those faces are shared between two adjacent solid opaque voxels and will never be seen by the camera. Emitting geometry for hidden faces wastes vertex buffer memory, GPU bandwidth, and fragment shader cycles — a 32x32x32 chunk of solid stone would produce 196,608 triangles when only the surface shell (at most ~6,144 faces) is visible. The meshing pipeline must determine, for each voxel, which of its six faces are actually exposed to air or a transparent neighbor. This is the absolute foundation of all downstream meshing: only visible faces become geometry. Complicating matters, a voxel at the edge of a chunk has neighbors that live in a different chunk, so face visibility at chunk boundaries requires cross-chunk voxel lookups. Additionally, the voxel type registry defines whether a given voxel type is transparent or opaque, so visibility decisions must consult the registry rather than hardcoding material properties.

## Solution

Implement face visibility detection in the `nebula_meshing` crate as a function that takes a chunk's voxel data, access to adjacent chunks, and a reference to the voxel type registry, and produces a per-voxel bitmask of visible faces.

### Data Types

```rust
/// Bitmask indicating which of a voxel's 6 faces are visible.
/// Bit 0 = +X, Bit 1 = -X, Bit 2 = +Y, Bit 3 = -Y, Bit 4 = +Z, Bit 5 = -Z.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisibleFaces(pub u8);

impl VisibleFaces {
    pub const NONE: Self = Self(0);
    pub const ALL: Self = Self(0b0011_1111);

    pub fn is_visible(self, direction: FaceDirection) -> bool {
        self.0 & (1 << direction as u8) != 0
    }

    pub fn set_visible(&mut self, direction: FaceDirection) {
        self.0 |= 1 << direction as u8;
    }

    pub fn count(self) -> u32 {
        self.0.count_ones()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FaceDirection {
    PosX = 0,
    NegX = 1,
    PosY = 2,
    NegY = 3,
    PosZ = 4,
    NegZ = 5,
}
```

### Visibility Algorithm

```rust
pub fn compute_visible_faces(
    chunk: &ChunkVoxelData,
    neighbors: &ChunkNeighborhood,
    registry: &VoxelTypeRegistry,
) -> Vec<VisibleFaces> {
    let size = chunk.size(); // typically 32
    let mut result = vec![VisibleFaces::NONE; size * size * size];

    for z in 0..size {
        for y in 0..size {
            for x in 0..size {
                let voxel = chunk.get(x, y, z);
                if registry.is_air(voxel) {
                    continue; // air voxels produce no geometry
                }

                let mut faces = VisibleFaces::NONE;

                for dir in FaceDirection::ALL {
                    let (nx, ny, nz) = dir.offset(x as i32, y as i32, z as i32);
                    let neighbor_voxel = if chunk.in_bounds(nx, ny, nz) {
                        chunk.get(nx as usize, ny as usize, nz as usize)
                    } else {
                        neighbors.get(nx, ny, nz)
                    };

                    if registry.is_air(neighbor_voxel) || registry.is_transparent(neighbor_voxel) {
                        faces.set_visible(dir);
                    }
                }

                result[chunk.index(x, y, z)] = faces;
            }
        }
    }

    result
}
```

For each solid voxel, the algorithm checks all six neighbors. If the neighbor is air or a transparent type (as determined by the voxel type registry), that face is marked visible. Interior voxels surrounded entirely by solid opaque neighbors get `VisibleFaces::NONE` and are skipped by later meshing stages.

When the neighbor coordinate falls outside the chunk bounds (`nx < 0` or `nx >= size`, etc.), the `ChunkNeighborhood` is queried. If the adjacent chunk is not loaded, the neighborhood returns air by default, causing boundary faces to be visible. This is the conservative choice — it avoids invisible walls at the edge of loaded terrain.

### Transparent Voxel Handling

Transparent voxels (glass, water, leaves) do not hide the faces of their neighbors. A solid stone block next to a glass block will have that face marked visible. However, two adjacent glass blocks of the same type do hide their shared face to avoid z-fighting of transparent surfaces. Two transparent blocks of different types both show their shared face.

### Performance

The function iterates all voxels in the chunk once. For a 32x32x32 chunk (32,768 voxels), this is O(n) with a small constant factor — six neighbor lookups per voxel, each being an array index or a neighbor cache lookup. The result fits in 32,768 bytes (one `u8` per voxel).

## Outcome

The `nebula_meshing` crate exports `VisibleFaces`, `FaceDirection`, and `compute_visible_faces()`. Given a chunk and its neighborhood, the function produces a flat array of visibility bitmasks — one per voxel. Downstream meshing stages (greedy meshing, AO computation) consume this array to generate only the geometry that matters. Running `cargo test -p nebula_meshing` passes all visibility tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Only exposed voxel faces are rendered. Interior faces between two solid voxels are culled. The console logs `Faces: 1,024 visible of 49,152 total`, and the polygon count drops dramatically.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_voxel` | workspace | `ChunkVoxelData`, `VoxelTypeRegistry`, voxel type definitions |
| `nebula_cubesphere` | workspace | Chunk coordinate types shared across the engine |

No external crates required. The face detection is pure arithmetic on voxel arrays. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_empty_chunk(size: usize) -> ChunkVoxelData {
        ChunkVoxelData::new_filled(size, VoxelType::AIR)
    }

    fn make_solid_chunk(size: usize) -> ChunkVoxelData {
        ChunkVoxelData::new_filled(size, VoxelType::STONE)
    }

    fn default_registry() -> VoxelTypeRegistry {
        let mut reg = VoxelTypeRegistry::new();
        reg.register(VoxelType::AIR, VoxelProperties { transparent: true, ..Default::default() });
        reg.register(VoxelType::STONE, VoxelProperties { transparent: false, ..Default::default() });
        reg.register(VoxelType::GLASS, VoxelProperties { transparent: true, ..Default::default() });
        reg
    }

    /// A single solid voxel in an otherwise empty chunk should have all 6 faces visible.
    #[test]
    fn test_single_voxel_in_empty_chunk_has_six_visible_faces() {
        let mut chunk = make_empty_chunk(32);
        chunk.set(16, 16, 16, VoxelType::STONE);
        let neighbors = ChunkNeighborhood::all_air();
        let registry = default_registry();

        let faces = compute_visible_faces(&chunk, &neighbors, &registry);
        let vf = faces[chunk.index(16, 16, 16)];

        assert_eq!(vf.count(), 6);
        assert_eq!(vf, VisibleFaces::ALL);
    }

    /// Two adjacent solid opaque voxels share a face — that shared face is hidden on both.
    #[test]
    fn test_two_adjacent_solid_voxels_share_hidden_face() {
        let mut chunk = make_empty_chunk(32);
        chunk.set(10, 10, 10, VoxelType::STONE);
        chunk.set(11, 10, 10, VoxelType::STONE); // +X neighbor
        let neighbors = ChunkNeighborhood::all_air();
        let registry = default_registry();

        let faces = compute_visible_faces(&chunk, &neighbors, &registry);

        let vf_a = faces[chunk.index(10, 10, 10)];
        let vf_b = faces[chunk.index(11, 10, 10)];

        // Voxel A's +X face should be hidden
        assert!(!vf_a.is_visible(FaceDirection::PosX));
        // Voxel B's -X face should be hidden
        assert!(!vf_b.is_visible(FaceDirection::NegX));
        // Both should still have 5 visible faces
        assert_eq!(vf_a.count(), 5);
        assert_eq!(vf_b.count(), 5);
    }

    /// A transparent voxel (glass) does NOT hide the face of its solid neighbor.
    #[test]
    fn test_transparent_voxel_does_not_hide_neighbor_faces() {
        let mut chunk = make_empty_chunk(32);
        chunk.set(10, 10, 10, VoxelType::STONE);
        chunk.set(11, 10, 10, VoxelType::GLASS);
        let neighbors = ChunkNeighborhood::all_air();
        let registry = default_registry();

        let faces = compute_visible_faces(&chunk, &neighbors, &registry);

        let vf_stone = faces[chunk.index(10, 10, 10)];
        // Stone's +X face is next to glass (transparent) — still visible
        assert!(vf_stone.is_visible(FaceDirection::PosX));
        assert_eq!(vf_stone.count(), 6);
    }

    /// A boundary voxel (at x=0) should query the adjacent chunk for its -X neighbor.
    #[test]
    fn test_boundary_voxel_queries_adjacent_chunk() {
        let mut chunk = make_empty_chunk(32);
        chunk.set(0, 10, 10, VoxelType::STONE);

        // Neighbor chunk has a solid voxel at the adjacent position
        let mut neg_x_chunk = make_empty_chunk(32);
        neg_x_chunk.set(31, 10, 10, VoxelType::STONE);
        let neighbors = ChunkNeighborhood::with_neg_x(neg_x_chunk);
        let registry = default_registry();

        let faces = compute_visible_faces(&chunk, &neighbors, &registry);
        let vf = faces[chunk.index(0, 10, 10)];

        // -X face should be hidden because the neighbor chunk has a solid voxel there
        assert!(!vf.is_visible(FaceDirection::NegX));
        assert_eq!(vf.count(), 5);
    }

    /// A completely empty chunk should produce zero visible faces for every voxel.
    #[test]
    fn test_empty_chunk_produces_zero_faces() {
        let chunk = make_empty_chunk(32);
        let neighbors = ChunkNeighborhood::all_air();
        let registry = default_registry();

        let faces = compute_visible_faces(&chunk, &neighbors, &registry);

        for vf in &faces {
            assert_eq!(vf.count(), 0);
        }
    }

    /// Air voxels never produce visible faces regardless of their neighbors.
    #[test]
    fn test_air_voxel_has_no_visible_faces() {
        let chunk = make_empty_chunk(32);
        let neighbors = ChunkNeighborhood::all_air();
        let registry = default_registry();

        let faces = compute_visible_faces(&chunk, &neighbors, &registry);
        // Every voxel is air, so no faces anywhere
        assert!(faces.iter().all(|vf| *vf == VisibleFaces::NONE));
    }
}
```
