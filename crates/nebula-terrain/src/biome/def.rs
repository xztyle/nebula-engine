//! Biome definition: describes the properties of a single biome type.

use nebula_voxel::VoxelTypeId;

/// Full descriptor for a biome type.
#[derive(Clone, Debug)]
pub struct BiomeDef {
    /// Human-readable biome name (e.g., "temperate_forest").
    pub name: String,
    /// Voxel type placed on the terrain surface (e.g., grass, sand, snow).
    pub surface_voxel: VoxelTypeId,
    /// Voxel type for the layers immediately below the surface (e.g., dirt, sandstone).
    pub subsurface_voxel: VoxelTypeId,
    /// Probability of vegetation spawning per surface voxel, in `[0.0, 1.0]`.
    pub vegetation_density: f64,
    /// Identifier for the tree/plant archetype used in this biome. `None` for barren biomes.
    pub tree_type: Option<String>,
}
