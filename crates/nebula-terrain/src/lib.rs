//! Procedural terrain generation: multi-octave noise, biome assignment, and terrain generation pipeline.

mod async_generation;
mod cave;
mod feature;
mod heightmap;
mod ore;
mod terrain_height;

pub mod biome;
pub mod seed;

pub use async_generation::{
    AsyncChunkGenerator, GeneratedChunk, GenerationTask, generate_chunk_sync,
};
pub use biome::{
    BiomeDef, BiomeId, BiomeRegistry, BiomeRegistryError, BiomeSampler, WhittakerDiagram,
    WhittakerRegion,
};
pub use cave::{CaveCarver, CaveConfig};
pub use feature::{
    BiomeFeatureConfig, FeaturePlacer, FeatureTypeDef, FeatureTypeId, PlacedFeature,
    poisson_disk_2d,
};
pub use heightmap::{HeightmapParams, HeightmapSampler};
pub use ore::{OreDistribution, OreDistributor, default_ore_distributions};
pub use seed::{
    FixedPoint64, chunk_rng, derive_chunk_seed, det_atan2, det_cos, det_sin, det_sqrt,
    fbm_fixed_point, generate_and_hash, hash_chunk_data,
};
pub use terrain_height::{TerrainHeightConfig, TerrainHeightSampler, column_surface_height};
