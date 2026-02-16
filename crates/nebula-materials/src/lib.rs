//! Material system: PBR material definitions, GPU-friendly packed data, texture atlas, and material identifiers.

mod atlas;
mod material;

pub use atlas::{AtlasBuilder, AtlasConfig, AtlasError, TextureAtlas, VoxelTextures};
pub use material::{MaterialDef, MaterialError, MaterialGpuData, MaterialId};
