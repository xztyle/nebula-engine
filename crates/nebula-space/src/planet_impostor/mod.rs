//! Distant planet impostor rendering: camera-facing billboards with procedural
//! crescent shading driven by orbital mechanics.
//!
//! Renders distant planets as lit-sphere billboards with correct phase angle,
//! atmospheric rim glow, and distance-based angular scaling. Orbital positions
//! update over time using simplified Keplerian mechanics.

use bytemuck::{Pod, Zeroable};
use nebula_render::Camera;

mod orbital;
mod renderer;

pub use orbital::OrbitalElements;
pub use renderer::PlanetImpostorRenderer;

/// Data describing a distant planet for impostor rendering.
#[derive(Clone, Debug)]
pub struct DistantPlanet {
    /// Unique identifier for this planet.
    pub id: u64,
    /// Position in world space (128-bit coordinates).
    pub position: [i128; 3],
    /// Physical radius in engine units (meters).
    pub radius: f64,
    /// Albedo (reflectivity) in [0, 1]. Determines apparent brightness.
    pub albedo: f32,
    /// Surface color tint (linear RGB).
    pub color: [f32; 3],
    /// Whether the planet has an atmosphere (adds a thin colored rim).
    pub has_atmosphere: bool,
    /// Atmosphere color (linear RGB), only used if `has_atmosphere` is true.
    pub atmosphere_color: [f32; 3],
}

impl DistantPlanet {
    /// Compute the angular diameter in radians as seen from a given distance.
    pub fn angular_diameter(&self, distance: f64) -> f64 {
        if distance <= 0.0 {
            return std::f64::consts::PI;
        }
        2.0 * (self.radius / distance).atan()
    }

    /// Compute the apparent brightness as a fraction of full illumination.
    /// Uses the phase angle (angle between sun and observer as seen from the planet).
    pub fn phase_brightness(&self, phase_angle: f64) -> f32 {
        let lambert = ((1.0 + phase_angle.cos()) / 2.0) as f32;
        lambert * self.albedo
    }

    /// Compute the phase angle given the sun direction and observer direction
    /// (both as unit vectors from the planet's position).
    pub fn compute_phase_angle(to_sun: glam::DVec3, to_observer: glam::DVec3) -> f64 {
        let cos_phase = to_sun.normalize().dot(to_observer.normalize());
        cos_phase.clamp(-1.0, 1.0).acos()
    }

    /// Determine if this planet should be rendered as an impostor at a given distance.
    /// If the angular diameter is above ~0.5 degrees, use full geometry instead.
    pub fn should_render_as_impostor(&self, distance: f64) -> bool {
        self.angular_diameter(distance) < 0.0087
    }
}

/// GPU instance data for a single planet impostor billboard.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ImpostorInstance {
    /// Screen-space position of the billboard center (unit direction).
    pub position: [f32; 3],
    /// Billboard scale (angular radius mapped to screen units).
    pub scale: f32,
    /// Planet color.
    pub color: [f32; 3],
    /// Apparent brightness.
    pub brightness: f32,
    /// Sun direction in billboard-local space for crescent calculation.
    pub sun_dir_local: [f32; 3],
    /// Whether this planet has an atmosphere (0 or 1).
    pub has_atmosphere: u32,
    /// Atmosphere color.
    pub atmosphere_color: [f32; 3],
    /// Padding for 16-byte alignment.
    pub _padding: f32,
}

impl ImpostorInstance {
    /// Vertex buffer layout for instanced rendering.
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ImpostorInstance>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 2,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32,
                offset: 12,
                shader_location: 3,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 16,
                shader_location: 4,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32,
                offset: 28,
                shader_location: 5,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 32,
                shader_location: 6,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32,
                offset: 44,
                shader_location: 7,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 48,
                shader_location: 8,
            },
        ],
    };
}

/// GPU vertex for the billboard quad.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct ImpostorVertex {
    pub position: [f32; 2],
}

impl ImpostorVertex {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ImpostorVertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 0,
            shader_location: 0,
        }],
    };
}

/// Transform a sun direction into billboard-local space.
pub fn billboard_local_sun_dir(
    view_dir: glam::Vec3,
    sun_dir_world: glam::Vec3,
    camera: &Camera,
) -> glam::Vec3 {
    let cam_up = camera.up();
    let right = view_dir.cross(cam_up).normalize();
    let up = right.cross(view_dir).normalize();
    glam::Vec3::new(
        sun_dir_world.dot(right),
        sun_dir_world.dot(up),
        sun_dir_world.dot(view_dir),
    )
    .normalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    #[test]
    fn test_distant_planet_visible_as_dot() {
        let planet = DistantPlanet {
            id: 1,
            position: [0; 3],
            radius: 6_371_000.0,
            albedo: 0.3,
            color: [0.3, 0.5, 0.8],
            has_atmosphere: true,
            atmosphere_color: [0.4, 0.6, 1.0],
        };
        let distance = 149_597_870_700.0;
        let angular = planet.angular_diameter(distance);
        assert!(angular > 0.0);
        assert!(angular < 0.001);
    }

    #[test]
    fn test_crescent_shape_matches_sun_direction() {
        let phase = DistantPlanet::compute_phase_angle(DVec3::X, DVec3::NEG_Z);
        assert!((phase - std::f64::consts::FRAC_PI_2).abs() < 0.01);

        let full = DistantPlanet::compute_phase_angle(DVec3::NEG_Z, DVec3::NEG_Z);
        assert!(full < 0.01);

        let new = DistantPlanet::compute_phase_angle(DVec3::Z, DVec3::NEG_Z);
        assert!((new - std::f64::consts::PI).abs() < 0.01);
    }

    #[test]
    fn test_planet_angular_size_decreases_with_distance() {
        let planet = DistantPlanet {
            id: 1,
            position: [0; 3],
            radius: 6_371_000.0,
            albedo: 0.3,
            color: [0.3, 0.5, 0.8],
            has_atmosphere: false,
            atmosphere_color: [0.0; 3],
        };
        let near = planet.angular_diameter(100_000_000.0);
        let far = planet.angular_diameter(1_000_000_000.0);
        assert!(near > far);
        let ratio = near / far;
        assert!((ratio - 10.0).abs() < 0.5, "got {ratio}");
    }

    #[test]
    fn test_multiple_planets_render_simultaneously() {
        let planets: Vec<DistantPlanet> = (0..8)
            .map(|i| DistantPlanet {
                id: i as u64,
                position: [0; 3],
                radius: 5_000_000.0 + i as f64 * 1_000_000.0,
                albedo: 0.3,
                color: [0.5, 0.5, 0.5],
                has_atmosphere: i % 2 == 0,
                atmosphere_color: [0.3, 0.5, 0.8],
            })
            .collect();
        for planet in &planets {
            assert!(planet.should_render_as_impostor(500_000_000_000.0));
        }
    }

    #[test]
    fn test_phase_brightness_full_illumination() {
        let planet = DistantPlanet {
            id: 1,
            position: [0; 3],
            radius: 6_371_000.0,
            albedo: 0.5,
            color: [1.0; 3],
            has_atmosphere: false,
            atmosphere_color: [0.0; 3],
        };
        let bright = planet.phase_brightness(0.0);
        assert!((bright - 0.5).abs() < 1e-6);
        let dark = planet.phase_brightness(std::f64::consts::PI);
        assert!(dark < 0.01);
    }

    #[test]
    fn test_impostor_threshold_transition() {
        let planet = DistantPlanet {
            id: 1,
            position: [0; 3],
            radius: 6_371_000.0,
            albedo: 0.3,
            color: [0.5; 3],
            has_atmosphere: false,
            atmosphere_color: [0.0; 3],
        };
        assert!(planet.should_render_as_impostor(1e12));
        assert!(!planet.should_render_as_impostor(planet.radius * 10.0));
    }

    #[test]
    fn test_instance_data_alignment() {
        let size = std::mem::size_of::<ImpostorInstance>();
        assert_eq!(size % 16, 0, "size={size}");
    }
}
