//! Space rendering: procedural starfields, nebula volumetrics, skybox, and celestial body rendering.

pub mod nebula;
pub mod skybox;
pub mod starfield;
pub mod sun;

pub use nebula::{NebulaConfig, NebulaGenerator, NebulaLayer};
pub use skybox::SkyboxRenderer;
pub use starfield::{StarPoint, StarfieldCubemap, StarfieldGenerator, blackbody_to_rgb};
pub use sun::{StarType, SunProperties, SunRenderer};
