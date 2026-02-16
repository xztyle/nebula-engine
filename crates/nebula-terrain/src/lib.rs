//! Procedural terrain generation: multi-octave noise, biome assignment, and terrain generation pipeline.

mod heightmap;
mod terrain_height;

pub mod biome;

pub use biome::{
    BiomeDef, BiomeId, BiomeRegistry, BiomeRegistryError, BiomeSampler, WhittakerDiagram,
    WhittakerRegion,
};
pub use heightmap::{HeightmapParams, HeightmapSampler};
pub use terrain_height::{TerrainHeightConfig, TerrainHeightSampler, column_surface_height};
