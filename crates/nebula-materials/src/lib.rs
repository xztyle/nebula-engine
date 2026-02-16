//! Material system: PBR material definitions, GPU-friendly packed data, texture atlas,
//! material identifiers, and the unified material registry.

mod animator;
mod atlas;
pub mod blending;
mod material;
mod registry;

pub use animator::{AnimationGpuData, MaterialAnimation, MaterialAnimator};
pub use atlas::{AtlasBuilder, AtlasConfig, AtlasError, TextureAtlas, VoxelTextures};
pub use blending::{BiomeMap, blend_colors, compute_blend_weight, triplanar_weights};
pub use material::{MaterialDef, MaterialError, MaterialGpuData, MaterialId};
pub use registry::{
    Face, MaterialEntry, MaterialManifest, MaterialRegistry, MaterialUVs, RegistryError,
};
