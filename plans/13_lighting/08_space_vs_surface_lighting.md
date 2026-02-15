# Space vs Surface Lighting

## Problem

The Nebula Engine operates in two fundamentally different lighting environments: deep space and planetary surfaces. In space, there is no atmosphere to scatter light — the only illumination comes from the direct sun and point lights. Shadows in space are absolute black (as on the Moon). On a planet's surface, the atmosphere scatters sunlight into every direction, creating ambient sky light that softly illuminates shadows (as on Earth). A player transitioning from orbit to surface landing should see the lighting model change smoothly — shadows gradually filling with ambient light as the atmosphere thickens. Without this distinction, either space scenes look unrealistically bright (too much ambient) or surface scenes look unrealistically harsh (no fill light). The engine needs a unified lighting model that interpolates between these two regimes based on altitude.

## Solution

### Lighting Context

Define a `LightingContext` that encapsulates the environment-dependent parameters:

```rust
/// Environment-dependent lighting parameters.
#[derive(Clone, Debug)]
pub struct LightingContext {
    /// Ambient light color and intensity (sky-scattered light).
    /// Zero in space, positive on surface.
    pub ambient_color: glam::Vec3,
    /// Ambient intensity multiplier [0.0, 1.0].
    pub ambient_intensity: f32,
    /// Shadow darkness: 0.0 = fully black shadows, 1.0 = fully lit (no shadow).
    /// In space: 0.0. On surface: depends on atmosphere density.
    pub shadow_min_light: f32,
    /// Atmosphere density factor [0.0, 1.0]. 0 = vacuum, 1 = full atmosphere.
    pub atmosphere_density: f32,
}

impl LightingContext {
    pub fn space() -> Self {
        Self {
            ambient_color: glam::Vec3::ZERO,
            ambient_intensity: 0.0,
            shadow_min_light: 0.0,
            atmosphere_density: 0.0,
        }
    }

    pub fn earth_like_surface() -> Self {
        Self {
            ambient_color: glam::Vec3::new(0.4, 0.5, 0.7), // blue sky tint
            ambient_intensity: 0.15,
            shadow_min_light: 0.08,
            atmosphere_density: 1.0,
        }
    }
}
```

### Altitude-Based Transition

The transition between space and surface lighting is governed by altitude relative to the atmosphere's boundary. The atmosphere has a defined thickness (configurable per planet):

```rust
pub struct AtmosphereConfig {
    /// Altitude (in meters above the planet surface) where the atmosphere begins to thin.
    pub atmosphere_start: f64,
    /// Altitude where the atmosphere is effectively zero (full space).
    pub atmosphere_end: f64,
}

impl Default for AtmosphereConfig {
    fn default() -> Self {
        Self {
            atmosphere_start: 10_000.0,  // 10 km: full atmosphere below this
            atmosphere_end: 100_000.0,   // 100 km: full vacuum above this
        }
    }
}

/// Compute the lighting context for a given altitude above the planet surface.
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

    // Smooth interpolation between surface and space.
    let t = ((altitude - config.atmosphere_start)
        / (config.atmosphere_end - config.atmosphere_start)) as f32;
    // Use smoothstep for a perceptually smooth transition.
    let t = smoothstep(t);

    LightingContext {
        ambient_color: surface_context.ambient_color * (1.0 - t),
        ambient_intensity: surface_context.ambient_intensity * (1.0 - t),
        shadow_min_light: surface_context.shadow_min_light * (1.0 - t),
        atmosphere_density: surface_context.atmosphere_density * (1.0 - t),
    }
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
```

### GPU Uniform

The lighting context is uploaded as a uniform buffer each frame:

```rust
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightingContextUniform {
    /// xyz = ambient_color * ambient_intensity, w = shadow_min_light.
    pub ambient_shadow: [f32; 4],
    /// x = atmosphere_density, yzw = padding.
    pub atmosphere_padding: [f32; 4],
}
```

### Fragment Shader Integration

The fragment shader uses the lighting context to modulate ambient and shadow contributions:

```wgsl
struct LightingContext {
    ambient_shadow: vec4<f32>,    // xyz = ambient, w = shadow_min_light
    atmosphere_padding: vec4<f32>, // x = atmosphere_density
};

@group(1) @binding(2)
var<uniform> lighting_ctx: LightingContext;

fn apply_shadow(raw_shadow: f32) -> f32 {
    // In space, shadow_min_light = 0, so raw_shadow passes through (0 = black).
    // On surface, shadow_min_light > 0, so shadows are softly lit.
    return max(raw_shadow, lighting_ctx.ambient_shadow.w);
}

fn ambient_contribution(albedo: vec3<f32>, ao: f32) -> vec3<f32> {
    // In space, ambient = 0.
    // On surface, ambient provides fill light.
    return lighting_ctx.ambient_shadow.xyz * albedo * ao;
}
```

### Day/Night Sky Ambient

On the surface, the ambient color should also respond to the sun position. At night, the sky ambient drops significantly:

```rust
pub fn modulate_ambient_by_sun(
    base_ambient: glam::Vec3,
    sun_elevation: f32, // -1.0 (below horizon) to 1.0 (zenith)
) -> glam::Vec3 {
    let factor = (sun_elevation * 2.0 + 0.5).clamp(0.05, 1.0);
    base_ambient * factor
}
```

This ensures that nighttime on a planet surface still has a small amount of ambient (starlight, moonlight) but is significantly darker than daytime.

## Outcome

A `LightingContext` system in `nebula_lighting` that provides environment-appropriate ambient light and shadow behavior. Space has zero ambient and pure-black shadows. Surfaces have atmospheric ambient and softly-lit shadows. The transition is smooth and configurable. A GPU uniform delivers the context to the fragment shader each frame. Running `cargo test -p nebula_lighting` passes all space-vs-surface lighting tests. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

In orbit, the planet is lit by the distant sun with no atmospheric scattering. On the surface, atmospheric scattering adds ambient fill light. The transition is smooth.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | Uniform buffer for lighting context |
| `bytemuck` | `1.21` | Pod/Zeroable for GPU struct |
| `glam` | `0.29` | Vec3 for ambient color math |

Depends on story 01 (directional light) for sun direction and story 05 (PBR shading) for ambient term integration.

## Unit Tests

```rust
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
        assert!(ctx.ambient_intensity > 0.0, "surface ambient intensity must be positive");
        let effective = ctx.ambient_color * ctx.ambient_intensity;
        assert!(effective.x > 0.0, "surface ambient R should be positive");
        assert!(effective.y > 0.0, "surface ambient G should be positive");
        assert!(effective.z > 0.0, "surface ambient B should be positive");
    }

    #[test]
    fn test_shadow_darkness_matches_context() {
        let space = LightingContext::space();
        let surface = LightingContext::earth_like_surface();

        // In space, shadows are pure black (min_light = 0).
        assert_eq!(space.shadow_min_light, 0.0, "space shadows must be pure black");

        // On surface, shadows have some fill light.
        assert!(
            surface.shadow_min_light > 0.0,
            "surface shadows must have fill light"
        );
        assert!(
            surface.shadow_min_light < 1.0,
            "surface shadow min light must be less than 1.0 (not fully lit)"
        );
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

        // At 15,000m: config_a should be partially in space, config_b should be full surface.
        let ctx_a = lighting_context_at_altitude(15_000.0, &config_a, &surface);
        let ctx_b = lighting_context_at_altitude(15_000.0, &config_b, &surface);

        assert!(
            ctx_a.ambient_intensity < surface.ambient_intensity,
            "config_a at 15km should have reduced ambient"
        );
        assert_eq!(
            ctx_b.ambient_intensity, surface.ambient_intensity,
            "config_b at 15km should still have full surface ambient"
        );
    }

    #[test]
    fn test_both_models_produce_valid_output() {
        let space = LightingContext::space();
        let surface = LightingContext::earth_like_surface();

        // Both contexts should have finite, non-NaN values.
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

        // Ambient intensity should monotonically decrease as altitude increases.
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
}
```
