//! LOD-aware chunk data with variable resolution.
//!
//! At LOD N, a chunk stores `(32 / 2^N)^3` voxels covering the same spatial
//! extent as a full-resolution LOD 0 chunk. The terrain sampler evaluates at
//! coarser grid spacing, skipping intermediate voxels entirely.

use crate::chunk::CHUNK_SIZE;
use crate::registry::VoxelTypeId;

/// Maximum supported LOD level. LOD 5 gives 1×1×1 resolution.
pub const MAX_LOD: u8 = 5;

/// Compute the voxel grid resolution for a given LOD level.
///
/// LOD 0 = 32, LOD 1 = 16, LOD 2 = 8, LOD 3 = 4, LOD 4 = 2, LOD 5 = 1.
///
/// # Panics
///
/// Panics if `lod` exceeds [`MAX_LOD`] (5).
pub fn resolution_for_lod(lod: u8) -> u32 {
    assert!(
        lod <= MAX_LOD,
        "LOD level {lod} exceeds maximum ({MAX_LOD})"
    );
    (CHUNK_SIZE as u32) >> lod
}

/// Compute the world-space size of one voxel at a given LOD level.
///
/// At LOD 0 each voxel covers `base_voxel_size` meters.
/// At LOD N each voxel covers `base_voxel_size * 2^N` meters.
pub fn voxel_size_for_lod(lod: u8, base_voxel_size: f64) -> f64 {
    base_voxel_size * (1u64 << lod) as f64
}

/// A trait for sampling terrain at arbitrary world-space positions.
///
/// Implementations should evaluate a continuous density/terrain function
/// (e.g. noise-based) and return the voxel type at the given coordinates.
pub trait TerrainSampler {
    /// Sample the terrain at world-space coordinates `(wx, wy, wz)`.
    ///
    /// Returns the [`VoxelTypeId`] that should occupy the voxel at this position.
    fn sample(&self, wx: f64, wy: f64, wz: f64) -> VoxelTypeId;
}

/// Implements [`TerrainSampler`] for any closure `Fn(f64, f64, f64) -> VoxelTypeId`.
impl<F> TerrainSampler for F
where
    F: Fn(f64, f64, f64) -> VoxelTypeId,
{
    fn sample(&self, wx: f64, wy: f64, wz: f64) -> VoxelTypeId {
        self(wx, wy, wz)
    }
}

/// Chunk data with LOD-dependent resolution.
///
/// At LOD 0 the chunk holds 32³ voxels; at LOD N it holds `(32/2^N)³` voxels.
/// All LOD levels cover the same spatial extent (`32 * base_voxel_size`).
#[derive(Clone, Debug)]
pub struct LodChunkData {
    /// The LOD level of this chunk (0 = full resolution).
    lod: u8,
    /// Voxel grid resolution along each axis: `32 / 2^lod`.
    resolution: u32,
    /// Flat voxel storage indexed as `x + y * res + z * res * res`.
    voxels: Vec<VoxelTypeId>,
}

impl LodChunkData {
    /// Create a new chunk filled with air at the specified LOD level.
    pub fn new(lod: u8) -> Self {
        let resolution = resolution_for_lod(lod);
        let count = (resolution * resolution * resolution) as usize;
        Self {
            lod,
            resolution,
            voxels: vec![VoxelTypeId(0); count],
        }
    }

    /// Returns the LOD level of this chunk.
    pub fn lod(&self) -> u8 {
        self.lod
    }

    /// Returns the grid resolution along each axis.
    pub fn resolution(&self) -> u32 {
        self.resolution
    }

    /// Returns the voxel type at local grid coordinates.
    ///
    /// Coordinates must be in the range `[0, resolution)`.
    pub fn get(&self, x: u32, y: u32, z: u32) -> VoxelTypeId {
        debug_assert!(x < self.resolution && y < self.resolution && z < self.resolution);
        let index = (x + y * self.resolution + z * self.resolution * self.resolution) as usize;
        self.voxels[index]
    }

    /// Sets the voxel type at local grid coordinates.
    pub fn set(&mut self, x: u32, y: u32, z: u32, voxel: VoxelTypeId) {
        debug_assert!(x < self.resolution && y < self.resolution && z < self.resolution);
        let index = (x + y * self.resolution + z * self.resolution * self.resolution) as usize;
        self.voxels[index] = voxel;
    }

    /// Total number of voxels in this chunk.
    pub fn voxel_count(&self) -> usize {
        self.voxels.len()
    }

    /// The world-space extent of this chunk along each axis (same regardless of LOD).
    pub fn spatial_extent(&self, base_voxel_size: f64) -> f64 {
        CHUNK_SIZE as f64 * base_voxel_size
    }
}

/// Generate chunk voxel data at the specified LOD level.
///
/// The terrain sampler is evaluated at intervals of `base_voxel_size * 2^lod`
/// instead of `base_voxel_size`, producing a coarser grid covering the same area.
///
/// `chunk_origin` is the world-space position of the chunk's (0,0,0) corner
/// as `(x, y, z)` in f64.
pub fn generate_chunk_at_lod(
    chunk_origin: (f64, f64, f64),
    lod: u8,
    terrain: &dyn TerrainSampler,
    base_voxel_size: f64,
) -> LodChunkData {
    let resolution = resolution_for_lod(lod);
    let step = voxel_size_for_lod(lod, base_voxel_size);
    let mut chunk = LodChunkData::new(lod);

    for z in 0..resolution {
        for y in 0..resolution {
            for x in 0..resolution {
                let wx = chunk_origin.0 + x as f64 * step;
                let wy = chunk_origin.1 + y as f64 * step;
                let wz = chunk_origin.2 + z as f64 * step;

                let voxel = terrain.sample(wx, wy, wz);
                chunk.set(x, y, z, voxel);
            }
        }
    }

    chunk
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// LOD 0 chunk should have a resolution of 32 along each axis.
    #[test]
    fn test_lod_0_is_32_cubed() {
        let chunk = LodChunkData::new(0);
        assert_eq!(chunk.resolution(), 32);
        assert_eq!(chunk.voxel_count(), 32 * 32 * 32);
    }

    /// LOD 1 chunk should have a resolution of 16 along each axis.
    #[test]
    fn test_lod_1_is_16_cubed() {
        let chunk = LodChunkData::new(1);
        assert_eq!(chunk.resolution(), 16);
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
        let origin = (0.0, 0.0, 0.0);
        let base_voxel_size = 1.0;

        // Flat terrain: solid below y=16, air above.
        let flat_terrain = |_wx: f64, wy: f64, _wz: f64| -> VoxelTypeId {
            if wy < 16.0 {
                VoxelTypeId(1) // stone
            } else {
                VoxelTypeId(0) // air
            }
        };

        let lod0 = generate_chunk_at_lod(origin, 0, &flat_terrain, base_voxel_size);
        let lod1 = generate_chunk_at_lod(origin, 1, &flat_terrain, base_voxel_size);

        // Each LOD 1 voxel at (x, y, z) should match LOD 0 voxel at (2x, 2y, 2z).
        for z in 0..16u32 {
            for y in 0..16u32 {
                for x in 0..16u32 {
                    let lod1_voxel = lod1.get(x, y, z);
                    let lod0_voxel = lod0.get(x * 2, y * 2, z * 2);
                    assert_eq!(
                        lod1_voxel,
                        lod0_voxel,
                        "LOD 1 voxel at ({x},{y},{z}) should match LOD 0 at ({},{},{})",
                        x * 2,
                        y * 2,
                        z * 2
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

        assert!(
            count_0 > count_1,
            "LOD 0 should have more voxels than LOD 1"
        );
        assert!(
            count_1 > count_2,
            "LOD 1 should have more voxels than LOD 2"
        );
        assert_eq!(count_0, count_1 * 8, "each LOD step reduces voxels by 8x");
        assert_eq!(count_1, count_2 * 8, "each LOD step reduces voxels by 8x");
    }

    /// Voxel size doubles with each LOD level.
    #[test]
    fn test_voxel_size_doubles_per_lod() {
        let base = 1.0;
        for lod in 0..=5 {
            let expected = base * (1u64 << lod) as f64;
            let actual = voxel_size_for_lod(lod, base);
            assert!(
                (actual - expected).abs() < f64::EPSILON,
                "LOD {lod}: expected voxel size {expected}, got {actual}"
            );
        }
    }

    /// LOD 6 should panic.
    #[test]
    #[should_panic(expected = "LOD level 6 exceeds maximum")]
    fn test_lod_6_panics() {
        resolution_for_lod(6);
    }
}
