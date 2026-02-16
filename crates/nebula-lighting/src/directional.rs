//! Directional light: a single sun-like light source for the planet.
//!
//! The [`DirectionalLight`] struct describes the CPU-side light properties,
//! while [`DirectionalLightUniform`] is the GPU-side representation written
//! to a uniform buffer each frame.

use bytemuck::{Pod, Zeroable};

/// CPU-side directional light description.
///
/// Represents a single infinitely-distant light source (the sun). The direction
/// is in planet-local space so it rotates with the planet.
#[derive(Clone, Debug)]
pub struct DirectionalLight {
    /// Normalized direction vector pointing FROM the light (toward the surface).
    pub direction: glam::Vec3,
    /// Linear RGB color of the light (not premultiplied by intensity).
    pub color: glam::Vec3,
    /// Scalar intensity multiplier. Physical range is `[0.0, ..)`, typically 1.0–10.0.
    pub intensity: f32,
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self {
            // Sun slightly off-vertical for interesting initial shading.
            direction: glam::Vec3::new(0.0, -1.0, 0.0).normalize(),
            // Warm white, approximating D65 daylight.
            color: glam::Vec3::new(1.0, 0.96, 0.90),
            intensity: 1.0,
        }
    }
}

impl DirectionalLight {
    /// Set the light direction, normalizing the input.
    ///
    /// # Panics
    ///
    /// Panics if the input vector has near-zero length.
    pub fn set_direction(&mut self, dir: glam::Vec3) {
        let len = dir.length();
        assert!(len > 1e-6, "directional light direction must not be zero");
        self.direction = dir / len;
    }

    /// Build the GPU-side uniform from this light's properties.
    pub fn to_uniform(&self) -> DirectionalLightUniform {
        DirectionalLightUniform {
            direction_intensity: [
                self.direction.x,
                self.direction.y,
                self.direction.z,
                self.intensity,
            ],
            color_padding: [self.color.x, self.color.y, self.color.z, 0.0],
        }
    }
}

/// GPU-side representation, 32 bytes, std140-compatible.
///
/// Bound at `@group(1) @binding(0)` visible to `ShaderStages::FRAGMENT`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct DirectionalLightUniform {
    /// xyz = direction (normalized), w = intensity.
    pub direction_intensity: [f32; 4],
    /// xyz = color (linear RGB), w = padding.
    pub color_padding: [f32; 4],
}

/// Compute the sun direction at a given time by rotating a base direction
/// by the planet's rotation quaternion.
///
/// Returns a normalized direction vector.
pub fn sun_direction_at_time(
    base_direction: glam::Vec3,
    planet_rotation: glam::Quat,
) -> glam::Vec3 {
    (planet_rotation * base_direction).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_direction_is_normalized() {
        let light = DirectionalLight::default();
        let len = light.direction.length();
        assert!(
            (len - 1.0).abs() < 1e-6,
            "direction must be unit length, got {len}"
        );
    }

    #[test]
    fn test_set_direction_normalizes() {
        let mut light = DirectionalLight::default();
        light.set_direction(glam::Vec3::new(3.0, -4.0, 0.0));
        let len = light.direction.length();
        assert!(
            (len - 1.0).abs() < 1e-6,
            "set_direction must normalize, got {len}"
        );
    }

    #[test]
    #[should_panic(expected = "must not be zero")]
    fn test_zero_direction_panics() {
        let mut light = DirectionalLight::default();
        light.set_direction(glam::Vec3::ZERO);
    }

    #[test]
    fn test_uniform_buffer_layout_matches_shader() {
        // The GPU struct must be exactly 32 bytes (two vec4<f32>).
        assert_eq!(std::mem::size_of::<DirectionalLightUniform>(), 32);
        // Verify field offsets for std140 alignment.
        assert_eq!(
            std::mem::offset_of!(DirectionalLightUniform, direction_intensity),
            0
        );
        assert_eq!(
            std::mem::offset_of!(DirectionalLightUniform, color_padding),
            16
        );
    }

    #[test]
    fn test_default_sun_color_is_warm_white() {
        let light = DirectionalLight::default();
        assert!(light.color.x >= light.color.y, "R should be >= G");
        assert!(light.color.y >= light.color.z, "G should be >= B");
        assert!(light.color.x > 0.9, "R should be near 1.0");
        assert!(light.color.z > 0.8, "B should not be too dim");
    }

    #[test]
    fn test_intensity_in_valid_range() {
        let light = DirectionalLight::default();
        assert!(light.intensity > 0.0, "intensity must be positive");
        assert!(light.intensity.is_finite(), "intensity must be finite");
    }

    #[test]
    fn test_direction_updates_with_planet_rotation() {
        let base = glam::Vec3::new(0.0, -1.0, 0.0);
        // Rotate 90 degrees around Z axis.
        let rotation = glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let rotated = sun_direction_at_time(base, rotation);
        // After 90-degree Z rotation, (0,-1,0) becomes (1,0,0).
        assert!((rotated.x - 1.0).abs() < 1e-5);
        assert!(rotated.y.abs() < 1e-5);
        assert!(rotated.z.abs() < 1e-5);
        assert!((rotated.length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_to_uniform_packs_correctly() {
        let light = DirectionalLight {
            direction: glam::Vec3::new(0.0, -1.0, 0.0),
            color: glam::Vec3::new(1.0, 0.5, 0.25),
            intensity: 2.0,
        };
        let u = light.to_uniform();
        assert!((u.direction_intensity[1] - (-1.0)).abs() < 1e-6);
        assert!((u.direction_intensity[3] - 2.0).abs() < 1e-6);
        assert!((u.color_padding[0] - 1.0).abs() < 1e-6);
        assert!((u.color_padding[1] - 0.5).abs() < 1e-6);
        assert!((u.color_padding[2] - 0.25).abs() < 1e-6);
        assert!((u.color_padding[3] - 0.0).abs() < 1e-6);
    }
}
