//! Procedural terrain generation: multi-octave noise, biome assignment, and terrain generation pipeline.

mod heightmap;

pub mod biome;

pub use biome::{
    BiomeDef, BiomeId, BiomeRegistry, BiomeRegistryError, BiomeSampler, WhittakerDiagram,
    WhittakerRegion,
};
pub use heightmap::{HeightmapParams, HeightmapSampler};
