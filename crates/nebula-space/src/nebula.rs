//! Procedural nebula background generation: multi-layered fractal noise clouds
//! baked into the starfield cubemap for zero-cost runtime rendering.

use noise::{NoiseFn, Simplex};

use crate::starfield::StarfieldCubemap;

/// Configuration for a single nebula layer.
#[derive(Clone, Debug)]
pub struct NebulaLayer {
    /// Base color of this nebula layer in linear RGB.
    pub color: [f32; 3],
    /// Maximum opacity of this layer. Kept low (0.05–0.2) so stars show through.
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
                    color: [0.4, 0.1, 0.6], // purple
                    max_opacity: 0.12,
                    frequency: 1.5,
                    octaves: 5,
                    persistence: 0.45,
                    lacunarity: 2.2,
                    offset: glam::DVec3::new(0.0, 0.0, 0.0),
                },
                NebulaLayer {
                    color: [0.1, 0.2, 0.7], // blue
                    max_opacity: 0.10,
                    frequency: 2.0,
                    octaves: 4,
                    persistence: 0.5,
                    lacunarity: 2.0,
                    offset: glam::DVec3::new(100.0, 0.0, 0.0),
                },
                NebulaLayer {
                    color: [0.7, 0.2, 0.5], // pink
                    max_opacity: 0.08,
                    frequency: 2.5,
                    octaves: 4,
                    persistence: 0.4,
                    lacunarity: 2.3,
                    offset: glam::DVec3::new(0.0, 100.0, 0.0),
                },
                NebulaLayer {
                    color: [0.8, 0.4, 0.1], // orange
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

/// Generates procedural nebula cloud colors by sampling multi-layered 3D fractal noise.
pub struct NebulaGenerator {
    noise: Simplex,
    config: NebulaConfig,
}

impl NebulaGenerator {
    /// Create a new generator with the given configuration.
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
            let noise_val = self.noise.get([p.x, p.y, p.z]);
            total += noise_val * amplitude;
            frequency *= layer.lacunarity;
            amplitude *= layer.persistence;
        }

        // Remap from [-1, 1] to [0, 1].
        let normalized = ((total + 1.0) * 0.5).clamp(0.0, 1.0);

        // Apply wisp exponent to create sparse, filamentary structures.
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
    /// Returns RGBA in `[0, 1]` with premultiplied alpha, suitable for
    /// additive blending onto the starfield.
    pub fn sample(&self, direction: glam::DVec3) -> [f32; 4] {
        let mut result = [0.0_f32; 4];

        for layer in &self.config.layers {
            let layer_color = self.sample_layer(direction, layer);
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

                    // Additive blend: nebula glow adds to star background.
                    pixel[0] = (pixel[0] + nebula_color[0]).min(1.0);
                    pixel[1] = (pixel[1] + nebula_color[1]).min(1.0);
                    pixel[2] = (pixel[2] + nebula_color[2]).min(1.0);
                }
            }
        }
    }
}

/// Convert a cube face index and UV coordinates to a unit direction vector.
///
/// Face indices: 0=+X, 1=−X, 2=+Y, 3=−Y, 4=+Z, 5=−Z.
fn cube_face_uv_to_direction(face: usize, u: f32, v: f32) -> glam::Vec3 {
    // Remap UV from [0,1] to [-1,1]
    let uc = u * 2.0 - 1.0;
    let vc = v * 2.0 - 1.0;

    // Inverse of the mapping in direction_to_cube_face_uv
    let dir = match face {
        0 => glam::Vec3::new(1.0, -vc, -uc),  // +X
        1 => glam::Vec3::new(-1.0, -vc, uc),  // -X
        2 => glam::Vec3::new(uc, 1.0, vc),    // +Y
        3 => glam::Vec3::new(uc, -1.0, -vc),  // -Y
        4 => glam::Vec3::new(uc, -vc, 1.0),   // +Z
        5 => glam::Vec3::new(-uc, -vc, -1.0), // -Z
        _ => glam::Vec3::Z,
    };
    dir.normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nebula_generates_without_errors() {
        let config = NebulaConfig::default();
        let generator = NebulaGenerator::new(config);
        for i in 0..100 {
            let theta = (i as f64 / 100.0) * std::f64::consts::TAU;
            for j in 0..50 {
                let phi = (j as f64 / 50.0) * std::f64::consts::PI;
                let dir =
                    glam::DVec3::new(phi.sin() * theta.cos(), phi.sin() * theta.sin(), phi.cos());
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
            for (ch, &val) in color.iter().enumerate().take(3) {
                assert!(
                    (0.0..=1.0).contains(&val),
                    "Nebula color channel {ch} = {val} is outside [0, 1]",
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
            let dir = glam::DVec3::new(phi.sin() * theta.cos(), phi.sin() * theta.sin(), phi.cos());
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
        let config = NebulaConfig::default();
        let generator = NebulaGenerator::new(config);
        let mut opacities: Vec<f32> = Vec::new();
        for i in 0..2000 {
            let theta = (i as f64 / 2000.0) * std::f64::consts::TAU;
            let phi = (i as f64 * 0.618) * std::f64::consts::PI;
            let dir = glam::DVec3::new(phi.sin() * theta.cos(), phi.sin() * theta.sin(), phi.cos());
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
        let config_a = NebulaConfig {
            seed: 42,
            ..Default::default()
        };
        let config_b = NebulaConfig {
            seed: 42,
            ..Default::default()
        };
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
        let gen_a = NebulaGenerator::new(NebulaConfig {
            seed: 1,
            ..Default::default()
        });
        let gen_b = NebulaGenerator::new(NebulaConfig {
            seed: 9999,
            ..Default::default()
        });
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

    #[test]
    fn test_cube_face_uv_roundtrip() {
        use crate::starfield::direction_to_cube_face_uv;
        // Verify that direction_to_cube_face_uv and cube_face_uv_to_direction are consistent.
        let dirs = [
            glam::Vec3::new(1.0, 0.5, -0.3).normalize(),
            glam::Vec3::new(-0.2, 1.0, 0.7).normalize(),
            glam::Vec3::new(0.4, -0.6, 1.0).normalize(),
        ];
        for original in &dirs {
            let (face, u, v) = direction_to_cube_face_uv(*original);
            let reconstructed = cube_face_uv_to_direction(face, u, v);
            let dot = original.dot(reconstructed);
            assert!(
                dot > 0.99,
                "Roundtrip failed: original={original}, reconstructed={reconstructed}, dot={dot}"
            );
        }
    }

    #[test]
    fn test_apply_nebula_adds_color() {
        let stars = crate::StarfieldGenerator::new(42, 100).generate();
        let mut cubemap = StarfieldCubemap::render(&stars, 32);

        // Sum pixel values before nebula
        let sum_before: f32 = cubemap
            .faces
            .iter()
            .flat_map(|f| f.iter())
            .map(|p| p[0] + p[1] + p[2])
            .sum();

        let nebula = NebulaGenerator::new(NebulaConfig::default());
        cubemap.apply_nebula(&nebula);

        let sum_after: f32 = cubemap
            .faces
            .iter()
            .flat_map(|f| f.iter())
            .map(|p| p[0] + p[1] + p[2])
            .sum();

        assert!(
            sum_after > sum_before,
            "Nebula should add color: before={sum_before}, after={sum_after}"
        );
    }
}
