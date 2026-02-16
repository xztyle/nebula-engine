//! Procedural terrain generation: multi-octave noise, biome assignment, and terrain generation pipeline.

mod heightmap;

pub use heightmap::{HeightmapParams, HeightmapSampler};
