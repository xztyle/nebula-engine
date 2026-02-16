//! Light types, shadow mapping, PBR shading calculations, and ambient occlusion integration.

mod directional;
pub mod pbr;
mod point;
mod shadow;
pub mod voxel_light;

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
    ChunkLightMap, VoxelLight, propagate_block_light, propagate_sunlight, remove_block_light,
};
