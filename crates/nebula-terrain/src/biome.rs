//! Biome system: registry, Whittaker diagram lookup, and noise-based sampling.
//!
//! Assigns biomes to sphere-surface points using temperature/moisture noise fields
//! and a configurable Whittaker-style 2D lookup diagram.

mod def;
mod diagram;
mod registry;
mod sampler;

pub use def::BiomeDef;
pub use diagram::{WhittakerDiagram, WhittakerRegion};
pub use registry::{BiomeId, BiomeRegistry, BiomeRegistryError};
pub use sampler::BiomeSampler;
