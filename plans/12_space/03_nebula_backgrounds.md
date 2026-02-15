# Nebula Backgrounds

## Problem

A starfield alone produces a stark, empty sky. Real deep-space imagery shows colorful nebulae -- diffuse clouds of gas and dust in purple, blue, pink, and orange hues that add depth and visual richness to the cosmos. The engine needs procedurally generated nebula clouds rendered as part of the skybox background. Nebulae must be wispy and semi-transparent so stars remain visible through them. They should be deterministic (seeded by the universe seed), vary between different regions of space, and be rendered at zero ongoing cost by baking them into the skybox cubemap alongside the starfield. The nebula must not look like uniform fog or solid blobs -- it needs the filamentary, layered structure characteristic of emission and reflection nebulae.

## Solution

Implement a `NebulaGenerator` in the `nebula-space` crate that produces nebula cloud data by sampling multiple layers of 3D fractal noise on the sky sphere. Each layer uses different noise parameters and a distinct color, and the layers are composited with low opacity to create the characteristic wispy, multi-colored appearance. The nebula pixels are blended into the starfield cubemap before GPU upload.

### Nebula Configuration

```rust
/// Configuration for a single nebula layer.
#[derive(Clone, Debug)]
pub struct NebulaLayer {
    /// Base color of this nebula layer in linear RGB.
    pub color: [f32; 3],
    /// Maximum opacity of this layer. Kept low (0.05-0.2) so stars show through.
    pub max_opacity: f32,
    /// Base frequency of the noise for this layer. Lower = larger clouds.
    pub frequency: f64,
    /// Number of noise octaves for detail.
    pub octaves: u32,
    /// Persistence (amplitude decay per octave).
    pub persistence: f64,
    /// Lacunarity (frequency increase per octave).
    pub lacunarity: f64,
    /// Noise offset to distinguish this layer from others using the same seed.
    pub offset: glam::DVec3,
}

/// Configuration for the full nebula background.
#[derive(Clone, Debug)]
pub struct NebulaConfig {
    /// Universe seed for deterministic generation.
    pub seed: u64,
    /// Individual nebula color layers.
    pub layers: Vec<NebulaLayer>,
    /// Global opacity multiplier applied to all layers.
    pub global_opacity: f32,
    /// Power curve exponent applied to raw noise to create wispy falloff.
    /// Higher values produce sparser, more filamentary structures.
    pub wisp_exponent: f32,
}

impl Default for NebulaConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            layers: vec![
                NebulaLayer {
                    color: [0.4, 0.1, 0.6],  // purple
                    max_opacity: 0.12,
                    frequency: 1.5,
                    octaves: 5,
                    persistence: 0.45,
                    lacunarity: 2.2,
                    offset: glam::DVec3::new(0.0, 0.0, 0.0),
                },
                NebulaLayer {
                    color: [0.1, 0.2, 0.7],  // blue
                    max_opacity: 0.10,
                    frequency: 2.0,
                    octaves: 4,
                    persistence: 0.5,
                    lacunarity: 2.0,
                    offset: glam::DVec3::new(100.0, 0.0, 0.0),
                },
                NebulaLayer {
                    color: [0.7, 0.2, 0.5],  // pink
                    max_opacity: 0.08,
                    frequency: 2.5,
                    octaves: 4,
                    persistence: 0.4,
                    lacunarity: 2.3,
                    offset: glam::DVec3::new(0.0, 100.0, 0.0),
                },
                NebulaLayer {
                    color: [0.8, 0.4, 0.1],  // orange
                    max_opacity: 0.06,
                    frequency: 3.0,
                    octaves: 3,
                    persistence: 0.5,
                    lacunarity: 2.0,
                    offset: glam::DVec3::new(0.0, 0.0, 100.0),
                },
            ],
            global_opacity: 1.0,
            wisp_exponent: 3.0,
        }
    }
}
```

### Nebula Sampling

```rust
use noise::{NoiseFn, Simplex};

pub struct NebulaGenerator {
    noise: Simplex,
    config: NebulaConfig,
}

impl NebulaGenerator {
    pub fn new(config: NebulaConfig) -> Self {
        let noise = Simplex::new(config.seed as u32);
        Self { noise, config }
    }

    /// Sample a single nebula layer at a direction on the sky sphere.
    /// Returns an RGBA color with premultiplied alpha.
    fn sample_layer(&self, direction: glam::DVec3, layer: &NebulaLayer) -> [f32; 4] {
        let mut total = 0.0_f64;
        let mut frequency = layer.frequency;
        let mut amplitude = 1.0_f64;

        for _ in 0..layer.octaves {
            let p = direction * frequency + layer.offset;
            // Use 3D simplex noise to avoid UV seam artifacts on the sphere.
            let noise_val = self.noise.get([p.x, p.y, p.z]);
            total += noise_val * amplitude;

            frequency *= layer.lacunarity;
            amplitude *= layer.persistence;
        }

        // Remap from [-1, 1] to [0, 1].
        let normalized = ((total + 1.0) * 0.5).clamp(0.0, 1.0);

        // Apply wisp exponent to create sparse, filamentary structures.
        // High exponent squashes low values to near-zero, keeping only peaks.
        let wisped = (normalized as f32).powf(self.config.wisp_exponent);

        let opacity = wisped * layer.max_opacity * self.config.global_opacity;

        [
            layer.color[0] * opacity,
            layer.color[1] * opacity,
            layer.color[2] * opacity,
            opacity,
        ]
    }

    /// Sample all nebula layers at a sky direction and composite them.
    /// Returns RGBA in [0, 1] with premultiplied alpha, suitable for
    /// additive blending onto the starfield.
    pub fn sample(&self, direction: glam::DVec3) -> [f32; 4] {
        let mut result = [0.0_f32; 4];

        for layer in &self.config.layers {
            let layer_color = self.sample_layer(direction, layer);
            // Additive blend (premultiplied alpha).
            result[0] += layer_color[0];
            result[1] += layer_color[1];
            result[2] += layer_color[2];
            result[3] += layer_color[3];
        }

        // Clamp total opacity to prevent over-saturation.
        result[3] = result[3].clamp(0.0, 0.5);
        result
    }
}
```

### Cubemap Integration

The nebula is rendered into the same cubemap as the starfield (story 01). For each pixel in each cubemap face, the pixel direction is computed, the nebula is sampled, and the result is blended over the star background:

```rust
impl StarfieldCubemap {
    /// Blend nebula colors onto an existing starfield cubemap.
    pub fn apply_nebula(&mut self, nebula: &NebulaGenerator) {
        for face in 0..6 {
            for y in 0..self.face_size {
                for x in 0..self.face_size {
                    let u = (x as f32 + 0.5) / self.face_size as f32;
                    let v = (y as f32 + 0.5) / self.face_size as f32;
                    let direction = cube_face_uv_to_direction(face, u, v).as_dvec3();

                    let nebula_color = nebula.sample(direction);
                    let idx = (y * self.face_size + x) as usize;
                    let pixel = &mut self.faces[face][idx];

                    // Over compositing: nebula behind stars.
                    // Stars are opaque points on black, so additive works well.
                    pixel[0] += nebula_color[0];
                    pixel[1] += nebula_color[1];
                    pixel[2] += nebula_color[2];
                }
            }
        }
    }
}
```

### Artistic Constraints

The nebula generation enforces several artistic constraints to maintain visual quality:

1. **Maximum opacity cap**: No single pixel can have nebula opacity above 0.5, ensuring stars are always visible through the densest regions.
2. **Color range**: Layer colors are defined in the config and validated to be within the artistic palette (cool nebula tones: purples, blues, pinks, warm accents).
3. **Wisp exponent**: The power-curve creates the filamentary look by crushing mid-range noise values toward zero, leaving only the peaks as visible wisps.
4. **3D noise on the sphere**: By sampling 3D noise with the unit sphere direction as the coordinate (not 2D UV), there are no seam or pole artifacts.

## Outcome

A `NebulaGenerator` and `NebulaConfig` in `nebula-space` that produce deterministic, multi-layered nebula cloud backgrounds blended into the starfield cubemap. Running `cargo test -p nebula-space` passes all nebula generation tests. The nebula adds visual depth to the skybox without obscuring stars.

## Demo Integration

**Demo crate:** `nebula-demo`

Colorful nebula clouds appear in the skybox background, adding depth and visual interest. Stars remain visible through the translucent nebula layers.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `noise` | `0.9` | 3D simplex noise for volumetric cloud sampling |
| `glam` | `0.29` | DVec3 for sphere-surface direction vectors |

The nebula generator lives in the `nebula-space` crate alongside the starfield generator. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nebula_generates_without_errors() {
        let config = NebulaConfig::default();
        let generator = NebulaGenerator::new(config);
        // Sample a grid of directions and ensure no panics or NaN values.
        for i in 0..100 {
            let theta = (i as f64 / 100.0) * std::f64::consts::TAU;
            for j in 0..50 {
                let phi = (j as f64 / 50.0) * std::f64::consts::PI;
                let dir = glam::DVec3::new(
                    phi.sin() * theta.cos(),
                    phi.sin() * theta.sin(),
                    phi.cos(),
                );
                let color = generator.sample(dir);
                for (ch, &val) in color.iter().enumerate() {
                    assert!(
                        val.is_finite(),
                        "Nebula sample produced non-finite value at channel {ch}: {val}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_nebula_colors_within_artistic_range() {
        let config = NebulaConfig::default();
        let generator = NebulaGenerator::new(config);

        for i in 0..500 {
            let theta = (i as f64 / 500.0) * std::f64::consts::TAU;
            let dir = glam::DVec3::new(theta.cos(), theta.sin(), 0.5).normalize();
            let color = generator.sample(dir);

            // RGB channels should be non-negative and not excessively bright.
            for ch in 0..3 {
                assert!(
                    color[ch] >= 0.0 && color[ch] <= 1.0,
                    "Nebula color channel {ch} = {} is outside [0, 1]",
                    color[ch]
                );
            }
        }
    }

    #[test]
    fn test_nebula_opacity_allows_stars_to_show_through() {
        let config = NebulaConfig::default();
        let generator = NebulaGenerator::new(config);

        let mut max_opacity = 0.0_f32;
        for i in 0..1000 {
            let theta = (i as f64 / 1000.0) * std::f64::consts::TAU;
            let phi = (i as f64 / 1000.0) * std::f64::consts::PI;
            let dir = glam::DVec3::new(
                phi.sin() * theta.cos(),
                phi.sin() * theta.sin(),
                phi.cos(),
            );
            let color = generator.sample(dir);
            max_opacity = max_opacity.max(color[3]);
        }

        assert!(
            max_opacity <= 0.5,
            "Maximum nebula opacity ({max_opacity}) exceeds 0.5 -- stars would be obscured"
        );
    }

    #[test]
    fn test_noise_produces_wispy_patterns() {
        // Wispy patterns mean most samples have very low opacity (near zero),
        // with occasional higher values. The median opacity should be well below
        // the maximum possible opacity.
        let config = NebulaConfig::default();
        let generator = NebulaGenerator::new(config);

        let mut opacities: Vec<f32> = Vec::new();
        for i in 0..2000 {
            let theta = (i as f64 / 2000.0) * std::f64::consts::TAU;
            let phi = (i as f64 * 0.618) * std::f64::consts::PI; // golden ratio spacing
            let dir = glam::DVec3::new(
                phi.sin() * theta.cos(),
                phi.sin() * theta.sin(),
                phi.cos(),
            );
            let color = generator.sample(dir);
            opacities.push(color[3]);
        }

        opacities.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = opacities[opacities.len() / 2];
        let max = *opacities.last().unwrap();

        assert!(
            median < max * 0.3,
            "Median opacity ({median}) should be well below max ({max}) for wispy patterns"
        );
    }

    #[test]
    fn test_nebula_is_seeded_by_universe_seed() {
        let config_a = NebulaConfig { seed: 42, ..Default::default() };
        let config_b = NebulaConfig { seed: 42, ..Default::default() };
        let gen_a = NebulaGenerator::new(config_a);
        let gen_b = NebulaGenerator::new(config_b);

        let dir = glam::DVec3::new(0.5, 0.3, 0.8).normalize();
        let color_a = gen_a.sample(dir);
        let color_b = gen_b.sample(dir);

        for ch in 0..4 {
            assert!(
                (color_a[ch] - color_b[ch]).abs() < 1e-6,
                "Same seed should produce identical nebula: channel {ch} differs ({} vs {})",
                color_a[ch],
                color_b[ch]
            );
        }
    }

    #[test]
    fn test_different_seeds_produce_different_nebulae() {
        let gen_a = NebulaGenerator::new(NebulaConfig { seed: 1, ..Default::default() });
        let gen_b = NebulaGenerator::new(NebulaConfig { seed: 9999, ..Default::default() });

        let dir = glam::DVec3::new(0.5, 0.3, 0.8).normalize();
        let color_a = gen_a.sample(dir);
        let color_b = gen_b.sample(dir);

        let diff: f32 = (0..4).map(|ch| (color_a[ch] - color_b[ch]).abs()).sum();
        assert!(
            diff > 0.001,
            "Different seeds should produce different nebulae, total diff = {diff}"
        );
    }

    #[test]
    fn test_default_config_has_four_layers() {
        let config = NebulaConfig::default();
        assert_eq!(
            config.layers.len(),
            4,
            "Default config should have 4 nebula layers (purple, blue, pink, orange)"
        );
    }

    #[test]
    fn test_layer_max_opacity_is_low() {
        let config = NebulaConfig::default();
        for (i, layer) in config.layers.iter().enumerate() {
            assert!(
                layer.max_opacity <= 0.2,
                "Layer {i} max_opacity ({}) exceeds 0.2 -- too opaque",
                layer.max_opacity
            );
        }
    }
}
```
