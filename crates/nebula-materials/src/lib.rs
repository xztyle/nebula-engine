//! Material system: PBR material definitions, GPU-friendly packed data, texture atlas,
//! material identifiers, and the unified material registry.

mod atlas;
mod material;
mod registry;

pub use atlas::{AtlasBuilder, AtlasConfig, AtlasError, TextureAtlas, VoxelTextures};
pub use material::{MaterialDef, MaterialError, MaterialGpuData, MaterialId};
pub use registry::{
    Face, MaterialEntry, MaterialManifest, MaterialRegistry, MaterialUVs, RegistryError,
};
