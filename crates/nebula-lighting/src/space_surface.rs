//! Space vs surface lighting: environment-dependent ambient and shadow behavior.
//!
//! In space, there is no atmosphere to scatter light — shadows are pure black
//! and ambient light is zero. On a planet's surface, atmospheric scattering
//! provides fill light that softens shadows. This module provides
//! [`LightingContext`] to interpolate between these two regimes based on
//! altitude, and [`LightingContextUniform`] for GPU upload.

use bytemuck::{Pod, Zeroable};

/// Environment-dependent lighting parameters.
///
/// Encapsulates ambient light, shadow behavior, and atmosphere density
/// for a given position in the world. Interpolates smoothly between
/// deep-space (zero ambient, black shadows) and planetary surface
/// (atmospheric ambient, soft shadows).
#[derive(Clone, Debug)]
pub struct LightingContext {
    /// Ambient light color (sky-scattered light). Zero in space.
    pub ambient_color: glam::Vec3,
    /// Ambient intensity multiplier \[0.0, 1.0\].
    pub ambient_intensity: f32,
    /// Shadow darkness: 0.0 = fully black shadows, 1.0 = fully lit (no shadow).
    /// In space: 0.0. On surface: depends on atmosphere density.
    pub shadow_min_light: f32,
    /// Atmosphere density factor \[0.0, 1.0\]. 0 = vacuum, 1 = full atmosphere.
    pub atmosphere_density: f32,
}

impl LightingContext {
    /// Deep-space lighting: zero ambient, pure-black shadows.
    pub fn space() -> Self {
        Self {
            ambient_color: glam::Vec3::ZERO,
            ambient_intensity: 0.0,
            shadow_min_light: 0.0,
            atmosphere_density: 0.0,
        }
    }

    /// Earth-like surface lighting: blue sky tint, soft shadows.
    pub fn earth_like_surface() -> Self {
        Self {
            ambient_color: glam::Vec3::new(0.4, 0.5, 0.7),
            ambient_intensity: 0.15,
            shadow_min_light: 0.08,
            atmosphere_density: 1.0,
        }
    }

    /// Build the GPU-side uniform from this context.
    pub fn to_uniform(&self) -> LightingContextUniform {
        LightingContextUniform {
            ambient_shadow: [
                self.ambient_color.x * self.ambient_intensity,
                self.ambient_color.y * self.ambient_intensity,
                self.ambient_color.z * self.ambient_intensity,
                self.shadow_min_light,
            ],
            atmosphere_padding: [self.atmosphere_density, 0.0, 0.0, 0.0],
        }
    }
}

/// Configuration for altitude-based atmosphere transition.
#[derive(Clone, Debug)]
pub struct AtmosphereConfig {
    /// Altitude (meters) below which full atmosphere applies.
    pub atmosphere_start: f64,
    /// Altitude (meters) above which full vacuum applies.
    pub atmosphere_end: f64,
}

impl Default for AtmosphereConfig {
    fn default() -> Self {
        Self {
            atmosphere_start: 10_000.0,
            atmosphere_end: 100_000.0,
        }
    }
}

/// Smoothstep interpolation: 3t² − 2t³ for t in \[0, 1\].
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Compute the lighting context for a given altitude above the planet surface.
///
/// Smoothly interpolates between `surface_context` (below `atmosphere_start`)
/// and [`LightingContext::space()`] (above `atmosphere_end`).
pub fn lighting_context_at_altitude(
    altitude: f64,
    config: &AtmosphereConfig,
    surface_context: &LightingContext,
) -> LightingContext {
    if altitude <= config.atmosphere_start {
        return surface_context.clone();
    }
    if altitude >= config.atmosphere_end {
        return LightingContext::space();
    }

    let t = ((altitude - config.atmosphere_start)
        / (config.atmosphere_end - config.atmosphere_start)) as f32;
    let t = smoothstep(t);

    LightingContext {
        ambient_color: surface_context.ambient_color * (1.0 - t),
        ambient_intensity: surface_context.ambient_intensity * (1.0 - t),
        shadow_min_light: surface_context.shadow_min_light * (1.0 - t),
        atmosphere_density: surface_context.atmosphere_density * (1.0 - t),
    }
}

/// Modulate ambient color based on sun elevation.
///
/// At night, ambient drops to a minimum (starlight/moonlight).
/// `sun_elevation` ranges from −1.0 (below horizon) to 1.0 (zenith).
pub fn modulate_ambient_by_sun(base_ambient: glam::Vec3, sun_elevation: f32) -> glam::Vec3 {
    let factor = (sun_elevation * 2.0 + 0.5).clamp(0.05, 1.0);
    base_ambient * factor
}

/// GPU-side lighting context uniform, 32 bytes, std140-compatible.
///
/// Uploaded each frame alongside the directional light and shadow uniforms.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct LightingContextUniform {
    /// xyz = ambient_color × ambient_intensity, w = shadow_min_light.
    pub ambient_shadow: [f32; 4],
    /// x = atmosphere_density, yzw = padding.
    pub atmosphere_padding: [f32; 4],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ambient_light_is_zero_in_space() {
        let ctx = LightingContext::space();
        assert_eq!(ctx.ambient_color, glam::Vec3::ZERO);
        assert_eq!(ctx.ambient_intensity, 0.0);
        let effective = ctx.ambient_color * ctx.ambient_intensity;
        assert_eq!(effective, glam::Vec3::ZERO);
    }

    #[test]
    fn test_ambient_light_is_positive_on_surface() {
        let ctx = LightingContext::earth_like_surface();
        assert!(ctx.ambient_intensity > 0.0);
        let effective = ctx.ambient_color * ctx.ambient_intensity;
        assert!(effective.x > 0.0);
        assert!(effective.y > 0.0);
        assert!(effective.z > 0.0);
    }

    #[test]
    fn test_shadow_darkness_matches_context() {
        let space = LightingContext::space();
        let surface = LightingContext::earth_like_surface();
        assert_eq!(space.shadow_min_light, 0.0);
        assert!(surface.shadow_min_light > 0.0);
        assert!(surface.shadow_min_light < 1.0);
    }

    #[test]
    fn test_transition_altitude_is_configurable() {
        let config_a = AtmosphereConfig {
            atmosphere_start: 5_000.0,
            atmosphere_end: 50_000.0,
        };
        let config_b = AtmosphereConfig {
            atmosphere_start: 20_000.0,
            atmosphere_end: 200_000.0,
        };
        let surface = LightingContext::earth_like_surface();

        let ctx_a = lighting_context_at_altitude(15_000.0, &config_a, &surface);
        let ctx_b = lighting_context_at_altitude(15_000.0, &config_b, &surface);

        assert!(ctx_a.ambient_intensity < surface.ambient_intensity);
        assert_eq!(ctx_b.ambient_intensity, surface.ambient_intensity);
    }

    #[test]
    fn test_both_models_produce_valid_output() {
        let space = LightingContext::space();
        let surface = LightingContext::earth_like_surface();

        for ctx in [&space, &surface] {
            assert!(ctx.ambient_color.x.is_finite());
            assert!(ctx.ambient_color.y.is_finite());
            assert!(ctx.ambient_color.z.is_finite());
            assert!(ctx.ambient_intensity.is_finite());
            assert!(ctx.shadow_min_light.is_finite());
            assert!(ctx.atmosphere_density.is_finite());
            assert!(ctx.ambient_intensity >= 0.0);
            assert!(ctx.shadow_min_light >= 0.0);
            assert!(ctx.atmosphere_density >= 0.0 && ctx.atmosphere_density <= 1.0);
        }
    }

    #[test]
    fn test_transition_is_smooth() {
        let config = AtmosphereConfig::default();
        let surface = LightingContext::earth_like_surface();
        let steps = 100;
        let mut prev_ambient = f32::MAX;

        for i in 0..=steps {
            let altitude = config.atmosphere_start
                + (config.atmosphere_end - config.atmosphere_start) * (i as f64 / steps as f64);
            let ctx = lighting_context_at_altitude(altitude, &config, &surface);
            assert!(
                ctx.ambient_intensity <= prev_ambient + 1e-6,
                "ambient must not increase with altitude: step {i}, ambient {} > prev {prev_ambient}",
                ctx.ambient_intensity
            );
            prev_ambient = ctx.ambient_intensity;
        }
    }

    #[test]
    fn test_gpu_uniform_struct_size() {
        assert_eq!(std::mem::size_of::<LightingContextUniform>(), 32);
    }

    #[test]
    fn test_modulate_ambient_by_sun_noon() {
        let base = glam::Vec3::new(0.4, 0.5, 0.7);
        let result = modulate_ambient_by_sun(base, 1.0);
        // factor = (1.0 * 2.0 + 0.5).clamp(0.05, 1.0) = 1.0
        assert!((result - base).length() < 1e-6);
    }

    #[test]
    fn test_modulate_ambient_by_sun_midnight() {
        let base = glam::Vec3::new(0.4, 0.5, 0.7);
        let result = modulate_ambient_by_sun(base, -1.0);
        // factor = (-1.0 * 2.0 + 0.5).clamp(0.05, 1.0) = 0.05
        let expected = base * 0.05;
        assert!((result - expected).length() < 1e-6);
    }

    #[test]
    fn test_uniform_packs_correctly() {
        let ctx = LightingContext::earth_like_surface();
        let u = ctx.to_uniform();
        let expected_r = ctx.ambient_color.x * ctx.ambient_intensity;
        assert!((u.ambient_shadow[0] - expected_r).abs() < 1e-6);
        assert!((u.ambient_shadow[3] - ctx.shadow_min_light).abs() < 1e-6);
        assert!((u.atmosphere_padding[0] - ctx.atmosphere_density).abs() < 1e-6);
    }
}
