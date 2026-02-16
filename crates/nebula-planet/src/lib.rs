//! Planet-level rendering: single and six-face terrain loading, cubesphere displacement, and camera setup.

pub mod atmosphere;
mod culling;
pub mod day_night;
pub mod orbital;
mod origin;
mod single_face;
mod six_face;
mod transition;

pub use atmosphere::{AtmosphereParams, AtmosphereRenderer, AtmosphereUniform};
pub use culling::{CullResult, LocalFrustum, PlanetBounds};
pub use day_night::{
    DayNightClock, DayNightState, ambient_intensity, star_visibility, sun_color,
    sun_direction_from_time, sun_intensity_curve,
};
pub use orbital::{
    OrbitalMesh, OrbitalPipeline, OrbitalRenderer, PlanetUniform, generate_orbital_sphere,
    generate_terrain_color_texture, orbital_model_matrix,
};
pub use origin::OriginManager;
pub use single_face::{
    FaceChunkMesh, SingleFaceLoader, SingleFaceRenderData, build_face_render_data,
    create_face_camera,
};
pub use six_face::{FaceState, PlanetFaces, create_orbit_camera};
pub use transition::{TransitionConfig, TransitionUniform, chunk_budget_for_altitude};
