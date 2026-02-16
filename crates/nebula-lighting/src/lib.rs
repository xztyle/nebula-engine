//! Light types, shadow mapping, PBR shading calculations, and ambient occlusion integration.

pub mod cross_chunk;
mod directional;
pub mod pbr;
mod point;
mod shadow;
pub mod voxel_light;

pub use cross_chunk::{
    BorderLightFace, ChunkBorderLights, Face, border_changed, propagate_cross_chunk,
};
pub use directional::{DirectionalLight, DirectionalLightUniform, sun_direction_at_time};
pub use pbr::{PbrMaterial, PbrMaterialUniform};
pub use point::{
    Frustum as PointLightFrustum, PointLight, PointLightGpu, PointLightHeader, PointLightManager,
    attenuation,
};
pub use shadow::{
    CascadedShadowConfig, CascadedShadowMaps, ShadowUniform, compute_cascade_matrix,
    compute_cascade_matrix_from_camera,
};
pub use voxel_light::{
    ChunkLightMap, VoxelLight, collect_emissive_sources, propagate_block_light, propagate_sunlight,
    remove_block_light,
};
