//! Atmosphere scattering: Rayleigh + Mie single-scattering model.
//!
//! Provides [`AtmosphereParams`] for CPU-side configuration,
//! [`AtmosphereUniform`] for GPU upload, and [`AtmosphereRenderer`]
//! for the full-screen post-pass that composites atmosphere over terrain.

mod renderer;
mod scatter;

pub use renderer::{ATMOSPHERE_SHADER_SOURCE, AtmosphereRenderer};
pub use scatter::{
    AtmosphereParams, AtmosphereUniform, compute_single_scatter, ray_sphere_intersect_f32,
};
