//! Orbital-distance planet rendering: icosphere mesh, terrain color texture, and GPU pipeline.
//!
//! When the camera is far from the planet (orbital distance), individual voxels
//! are sub-pixel. This module renders the planet as a smooth textured sphere
//! with terrain colors derived from heightmap + biome data.

mod mesh;
mod pipeline;
pub mod texture;

pub use mesh::{OrbitalMesh, generate_orbital_sphere};
pub use pipeline::{
    ORBITAL_SHADER_SOURCE, OrbitalPipeline, OrbitalRenderer, PlanetUniform, orbital_model_matrix,
};
pub use texture::generate_terrain_color_texture;
