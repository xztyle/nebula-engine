//! Procedural terrain generation: multi-octave noise, biome assignment, and terrain generation pipeline.

mod async_generation;
mod cave;
mod feature;
mod heightmap;
mod ore;
mod terrain_height;

pub mod biome;

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
pub use terrain_height::{TerrainHeightConfig, TerrainHeightSampler, column_surface_height};
