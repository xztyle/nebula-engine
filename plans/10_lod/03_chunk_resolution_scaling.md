# Chunk Resolution Scaling

## Problem

When a chunk is assigned a LOD level greater than 0, it must contain fewer voxels while covering the same spatial extent. A naive approach would be to generate the full 32x32x32 chunk at LOD 0 and then downsample it (averaging or subsampling to produce 16x16x16 at LOD 1, etc.), but this defeats the purpose of LOD — the engine would still pay the full cost of generating 32,768 voxels for every chunk, even distant ones that only need 64 (4x4x4 at LOD 3). Instead, the terrain generator must directly produce reduced-resolution data by sampling the terrain function at the coarser grid spacing. This means the terrain noise function is evaluated at fewer points, the chunk allocates less memory, and the entire generation pipeline scales proportionally with the actual data size.

## Solution

Implement resolution-aware chunk generation in the `nebula_voxel` crate. At LOD N, a chunk stores `(32 / 2^N)^3` voxels. The terrain generator's sample spacing is multiplied by `2^N`, so it evaluates the noise function at wider intervals while covering the same world-space region.

### Resolution Mapping

```rust
/// Compute the voxel grid resolution for a given LOD level.
/// LOD 0 = 32^3, LOD 1 = 16^3, LOD 2 = 8^3, LOD 3 = 4^3, LOD 4 = 2^3, LOD 5 = 1^3.
pub fn resolution_for_lod(lod: u8) -> u32 {
    assert!(lod <= 5, "LOD level {lod} exceeds maximum (5)");
    32 >> lod
}

/// Compute the world-space size of one voxel at a given LOD level.
/// At LOD 0, each voxel covers 1 meter. At LOD 1, each voxel covers 2 meters, etc.
pub fn voxel_size_for_lod(lod: u8, base_voxel_size: f64) -> f64 {
    base_voxel_size * (1 << lod) as f64
}
```

### LOD-Aware Chunk Data

```rust
/// Chunk data with LOD-dependent resolution.
pub struct LodChunkData {
    /// The LOD level of this chunk (0 = full resolution).
    lod: u8,
    /// Voxel grid resolution along each axis: 32 / 2^lod.
    resolution: u32,
    /// Palette-compressed voxel storage (same scheme as ChunkData, but variable size).
    data: PaletteStorage,
}

impl LodChunkData {
    /// Create a new empty chunk at the specified LOD level.
    pub fn new(lod: u8) -> Self {
        let resolution = resolution_for_lod(lod);
        Self {
            lod,
            resolution,
            data: PaletteStorage::new_uniform(resolution as usize, VoxelTypeId::AIR),
        }
    }

    /// Get the voxel at local grid coordinates.
    /// Coordinates are in the range [0, resolution).
    pub fn get(&self, x: u32, y: u32, z: u32) -> VoxelTypeId {
        debug_assert!(x < self.resolution && y < self.resolution && z < self.resolution);
        let index = (x + y * self.resolution + z * self.resolution * self.resolution) as usize;
        self.data.get(index)
    }

    /// Set the voxel at local grid coordinates.
    pub fn set(&mut self, x: u32, y: u32, z: u32, voxel: VoxelTypeId) {
        debug_assert!(x < self.resolution && y < self.resolution && z < self.resolution);
        let index = (x + y * self.resolution + z * self.resolution * self.resolution) as usize;
        self.data.set(index, voxel);
    }

    /// Total number of voxels in this chunk.
    pub fn voxel_count(&self) -> usize {
        (self.resolution * self.resolution * self.resolution) as usize
    }

    /// The world-space extent of this chunk along each axis (same regardless of LOD).
    pub fn spatial_extent(&self, base_voxel_size: f64) -> f64 {
        32.0 * base_voxel_size
    }
}
```

### LOD-Aware Terrain Generation

```rust
/// Generate chunk voxel data at the specified LOD level.
/// The terrain function is sampled at intervals of `base_voxel_size * 2^lod`
/// instead of `base_voxel_size`, producing a coarser grid that covers the same area.
pub fn generate_chunk_at_lod(
    chunk_origin: &WorldPosition,
    lod: u8,
    terrain: &dyn TerrainGenerator,
    base_voxel_size: f64,
) -> LodChunkData {
    let resolution = resolution_for_lod(lod);
    let step = voxel_size_for_lod(lod, base_voxel_size);
    let mut chunk = LodChunkData::new(lod);

    for z in 0..resolution {
        for y in 0..resolution {
            for x in 0..resolution {
                // Sample position in world space
                let wx = chunk_origin.x as f64 + x as f64 * step;
                let wy = chunk_origin.y as f64 + y as f64 * step;
                let wz = chunk_origin.z as f64 + z as f64 * step;

                let voxel = terrain.sample(wx, wy, wz);
                chunk.set(x, y, z, voxel);
            }
        }
    }

    chunk
}
```

The critical insight is that the terrain generator itself is a continuous function (typically Perlin or simplex noise combined with density functions). It can be evaluated at any point in space regardless of grid spacing. By sampling at wider intervals, the engine skips intermediate voxels entirely — they are never computed, never allocated, never stored.

### Memory Savings

| LOD | Resolution | Voxels | Memory (palette, 2-bit) | Ratio vs LOD 0 |
|-----|-----------|--------|------------------------|----------------|
| 0 | 32^3 | 32,768 | 8,192 bytes | 1.0x |
| 1 | 16^3 | 4,096 | 1,024 bytes | 8x smaller |
| 2 | 8^3 | 512 | 128 bytes | 64x smaller |
| 3 | 4^3 | 64 | 16 bytes | 512x smaller |
| 4 | 2^3 | 8 | 2 bytes | 4096x smaller |

## Outcome

The `nebula_voxel` crate exports `LodChunkData`, `resolution_for_lod()`, `voxel_size_for_lod()`, and `generate_chunk_at_lod()`. Chunks at any LOD level cover the same spatial area but use proportionally fewer voxels and less memory. Running `cargo test -p nebula_voxel` passes all resolution-scaling tests. The terrain generator is invoked at the correct sample spacing, avoiding wasted computation on voxels that would be discarded.

## Demo Integration

**Demo crate:** `nebula-demo`

LOD 0 chunks contain 32x32x32 voxels; LOD 3 chunks contain 4x4x4 voxels covering the same spatial volume. Memory usage drops proportionally with distance.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_math` | workspace | `WorldPosition` (128-bit coordinates) |
| `nebula_terrain` | workspace | `TerrainGenerator` trait for terrain sampling |

No external crates required. Resolution scaling is simple integer arithmetic and index calculations. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// LOD 0 chunk should have a resolution of 32 along each axis.
    #[test]
    fn test_lod_0_is_32_cubed() {
        let chunk = LodChunkData::new(0);
        assert_eq!(chunk.resolution, 32);
        assert_eq!(chunk.voxel_count(), 32 * 32 * 32);
    }

    /// LOD 1 chunk should have a resolution of 16 along each axis.
    #[test]
    fn test_lod_1_is_16_cubed() {
        let chunk = LodChunkData::new(1);
        assert_eq!(chunk.resolution, 16);
        assert_eq!(chunk.voxel_count(), 16 * 16 * 16);
    }

    /// LOD N chunk should have resolution 32 / 2^N.
    #[test]
    fn test_lod_n_resolution_formula() {
        for lod in 0..=5 {
            let expected = 32u32 >> lod;
            assert_eq!(
                resolution_for_lod(lod),
                expected,
                "LOD {lod} should have resolution {expected}"
            );
        }
    }

    /// All LOD levels should produce chunks covering the same spatial extent.
    #[test]
    fn test_spatial_extent_same_regardless_of_lod() {
        let base_voxel_size = 1.0;
        let expected_extent = 32.0 * base_voxel_size;

        for lod in 0..=5 {
            let chunk = LodChunkData::new(lod);
            let extent = chunk.spatial_extent(base_voxel_size);
            assert!(
                (extent - expected_extent).abs() < f64::EPSILON,
                "LOD {lod} spatial extent should be {expected_extent}, got {extent}"
            );
        }
    }

    /// A low-res chunk should match the subsampled high-res chunk
    /// when both sample the same terrain function.
    #[test]
    fn test_low_res_matches_subsampled_high_res() {
        let origin = WorldPosition::new(0, 0, 0);
        let terrain = FlatTerrainGenerator::new(16.0); // solid below y=16, air above
        let base_voxel_size = 1.0;

        let lod0 = generate_chunk_at_lod(&origin, 0, &terrain, base_voxel_size);
        let lod1 = generate_chunk_at_lod(&origin, 1, &terrain, base_voxel_size);

        // LOD 1 samples every 2nd voxel from the same terrain function.
        // Each LOD 1 voxel at (x, y, z) should match LOD 0 voxel at (2x, 2y, 2z).
        for z in 0..16u32 {
            for y in 0..16u32 {
                for x in 0..16u32 {
                    let lod1_voxel = lod1.get(x, y, z);
                    let lod0_voxel = lod0.get(x * 2, y * 2, z * 2);
                    assert_eq!(
                        lod1_voxel, lod0_voxel,
                        "LOD 1 voxel at ({x},{y},{z}) should match LOD 0 at ({},{},{})",
                        x * 2, y * 2, z * 2
                    );
                }
            }
        }
    }

    /// Generation time should decrease with higher LOD levels because fewer
    /// voxels are sampled.
    #[test]
    fn test_fewer_voxels_at_higher_lod() {
        let count_0 = LodChunkData::new(0).voxel_count();
        let count_1 = LodChunkData::new(1).voxel_count();
        let count_2 = LodChunkData::new(2).voxel_count();

        assert!(count_0 > count_1, "LOD 0 should have more voxels than LOD 1");
        assert!(count_1 > count_2, "LOD 1 should have more voxels than LOD 2");
        assert_eq!(count_0, count_1 * 8, "each LOD step reduces voxels by 8x");
        assert_eq!(count_1, count_2 * 8, "each LOD step reduces voxels by 8x");
    }
}
```
