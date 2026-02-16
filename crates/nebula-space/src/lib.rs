//! Space rendering: procedural starfields, nebula volumetrics, skybox, and celestial body rendering.

pub mod skybox;
pub mod starfield;

pub use skybox::SkyboxRenderer;
pub use starfield::{StarPoint, StarfieldCubemap, StarfieldGenerator, blackbody_to_rgb};
