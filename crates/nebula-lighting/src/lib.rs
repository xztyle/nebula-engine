//! Light types, shadow mapping, PBR shading calculations, and ambient occlusion integration.

mod directional;
mod point;
mod shadow;

pub use directional::{DirectionalLight, DirectionalLightUniform, sun_direction_at_time};
pub use point::{
    Frustum as PointLightFrustum, PointLight, PointLightGpu, PointLightHeader, PointLightManager,
    attenuation,
};
pub use shadow::{
    CascadedShadowConfig, CascadedShadowMaps, ShadowUniform, compute_cascade_matrix,
    compute_cascade_matrix_from_camera,
};
