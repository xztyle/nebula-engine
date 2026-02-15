# Procedural Starfield

## Problem

A space engine needs a convincing starfield background. Without stars, the skybox is a featureless black void that destroys any sense of being in space. The starfield must be procedurally generated from a universe seed so that every player in the same universe sees the same constellations. Stars must be distributed across the full sky sphere with realistic density variation (denser band for a galactic plane effect), varying brightness, and subtle color variation ranging from blue-white (hot stars) to red (cool stars). The starfield is effectively at infinite distance and should be rendered at maximum depth so that all other geometry draws in front of it. Regenerating the starfield every frame is wasteful -- it should be baked into a cubemap texture once and only regenerated when the player moves a significant distance (changing the visible stellar neighborhood) or when the universe seed changes.

## Solution

Implement a `StarfieldGenerator` in the `nebula-space` crate that produces a cubemap texture containing ~5000 procedurally placed stars. Each star is a point on the unit sphere with brightness, color, and angular size attributes. The generator writes star pixels directly into the six faces of a cubemap texture, which is then uploaded to the GPU and sampled by the skybox shader.

### Star Data

```rust
/// A single star in the procedural starfield catalog.
#[derive(Clone, Debug)]
pub struct StarPoint {
    /// Unit direction vector on the sky sphere.
    pub direction: glam::Vec3,
    /// Brightness in [0.0, 1.0] where 1.0 is the brightest visible star.
    pub brightness: f32,
    /// Color temperature mapped to RGB. Blue-white (high temp) to red (low temp).
    pub color: [f32; 3],
    /// Angular radius in radians. Most stars are sub-pixel; brighter ones are slightly larger.
    pub angular_radius: f32,
}
```

### Star Generation

```rust
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand::Rng;

pub struct StarfieldGenerator {
    seed: u64,
    star_count: u32,
}

impl StarfieldGenerator {
    pub fn new(seed: u64, star_count: u32) -> Self {
        Self { seed, star_count }
    }

    /// Generate the star catalog. Deterministic for a given seed.
    pub fn generate(&self) -> Vec<StarPoint> {
        let mut rng = ChaCha8Rng::seed_from_u64(self.seed);
        let mut stars = Vec::with_capacity(self.star_count as usize);

        for _ in 0..self.star_count {
            // Uniform random point on the unit sphere using spherical coordinates.
            // theta in [0, 2*PI), phi = acos(1 - 2*u) for uniform distribution.
            let theta = rng.gen::<f32>() * std::f32::consts::TAU;
            let phi = (1.0 - 2.0 * rng.gen::<f32>()).acos();

            let direction = glam::Vec3::new(
                phi.sin() * theta.cos(),
                phi.sin() * theta.sin(),
                phi.cos(),
            );

            // Brightness follows a power-law distribution: many dim stars, few bright ones.
            let raw: f32 = rng.gen();
            let brightness = raw.powf(3.0).clamp(0.0, 1.0);

            // Color temperature: map brightness to a rough spectral class.
            // Brighter stars skew blue-white, dimmer stars skew orange-red.
            let temperature = 2000.0 + brightness * 28000.0; // 2000K (red) to 30000K (blue)
            let color = blackbody_to_rgb(temperature);

            // Angular radius: most stars are sub-pixel, bright ones get a slight spread.
            let angular_radius = 0.0001 + brightness * 0.001;

            stars.push(StarPoint {
                direction,
                brightness,
                color,
                angular_radius,
            });
        }

        stars
    }
}

/// Attempt to convert a blackbody temperature in Kelvin to an approximate sRGB color.
/// Uses a simplified Planckian locus approximation.
fn blackbody_to_rgb(temperature_k: f32) -> [f32; 3] {
    let t = temperature_k / 100.0;
    let r = if t <= 66.0 {
        1.0
    } else {
        (329.698727446 * (t - 60.0).powf(-0.1332047592) / 255.0).clamp(0.0, 1.0)
    };
    let g = if t <= 66.0 {
        (99.4708025861 * t.ln() - 161.1195681661).clamp(0.0, 255.0) / 255.0
    } else {
        (288.1221695283 * (t - 60.0).powf(-0.0755148492) / 255.0).clamp(0.0, 1.0)
    };
    let b = if t >= 66.0 {
        1.0
    } else if t <= 19.0 {
        0.0
    } else {
        (138.5177312231 * (t - 10.0).ln() - 305.0447927307).clamp(0.0, 255.0) / 255.0
    };
    [r, g, b]
}
```

### Cubemap Rendering

```rust
pub struct StarfieldCubemap {
    pub face_size: u32,
    /// Six faces, each `face_size * face_size` pixels, stored as RGBA f32.
    pub faces: [[Vec<[f32; 4]>; 6]],
}

impl StarfieldCubemap {
    /// Render a star catalog into a cubemap texture.
    pub fn render(stars: &[StarPoint], face_size: u32) -> Self {
        let pixel_count = (face_size * face_size) as usize;
        let mut faces = std::array::from_fn(|_| vec![[0.0, 0.0, 0.0, 1.0]; pixel_count]);

        for star in stars {
            // Determine which cube face and UV coordinate the star direction maps to.
            let (face_index, u, v) = direction_to_cube_face_uv(star.direction);

            // Convert UV [0,1] to pixel coordinates.
            let px = (u * face_size as f32).min(face_size as f32 - 1.0) as u32;
            let py = (v * face_size as f32).min(face_size as f32 - 1.0) as u32;
            let idx = (py * face_size + px) as usize;

            // Additive blend: multiple dim stars in the same pixel accumulate brightness.
            let pixel = &mut faces[face_index][idx];
            pixel[0] += star.color[0] * star.brightness;
            pixel[1] += star.color[1] * star.brightness;
            pixel[2] += star.color[2] * star.brightness;
        }

        Self { face_size, faces }
    }
}

/// Map a unit direction vector to a cube face index (0..6) and UV coordinates in [0, 1].
fn direction_to_cube_face_uv(dir: glam::Vec3) -> (usize, f32, f32) {
    let abs = dir.abs();
    let (face, u, v) = if abs.x >= abs.y && abs.x >= abs.z {
        if dir.x > 0.0 {
            (0, -dir.z / abs.x, -dir.y / abs.x) // +X
        } else {
            (1, dir.z / abs.x, -dir.y / abs.x)  // -X
        }
    } else if abs.y >= abs.x && abs.y >= abs.z {
        if dir.y > 0.0 {
            (2, dir.x / abs.y, dir.z / abs.y)    // +Y
        } else {
            (3, dir.x / abs.y, -dir.z / abs.y)   // -Y
        }
    } else {
        if dir.z > 0.0 {
            (4, dir.x / abs.z, -dir.y / abs.z)   // +Z
        } else {
            (5, -dir.x / abs.z, -dir.y / abs.z)  // -Z
        }
    };
    // Remap from [-1, 1] to [0, 1].
    (face, u * 0.5 + 0.5, v * 0.5 + 0.5)
}
```

### Skybox Shader Integration

The cubemap is uploaded to the GPU as a `wgpu::Texture` with `TextureDimension::D2` and `TextureViewDimension::Cube` (six layers). The skybox shader samples it using the camera's inverse view direction:

```wgsl
@group(0) @binding(0)
var skybox_texture: texture_cube<f32>;
@group(0) @binding(1)
var skybox_sampler: sampler;

@fragment
fn fs_skybox(in: SkyboxVertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(skybox_texture, skybox_sampler, in.view_dir);
    return color;
}
```

The skybox is rendered with depth write disabled and depth test set to `LessEqual` against the clear depth value (0.0 in reverse-Z), ensuring it renders behind all scene geometry.

### Regeneration Strategy

The starfield cubemap is regenerated when the player crosses a sector boundary (defined by the 128-bit coordinate system's sector grid). A sector transition changes which stars are "nearby" and might shift the galactic plane orientation. Between sector transitions, the cubemap is static and costs zero per-frame GPU time beyond the skybox draw call.

## Outcome

A `StarfieldGenerator` and `StarfieldCubemap` in `nebula-space` that produce a deterministic, seed-based starfield baked into a cubemap texture. Running `cargo test -p nebula-space` passes all starfield generation tests. The cubemap integrates with the existing skybox rendering pipeline from the rendering story group.

## Demo Integration

**Demo crate:** `nebula-demo`

The skybox shows thousands of stars with varying brightness and color temperature. The starfield is identical across restarts with the same seed.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | `0.29` | Vec3 direction vectors and math |
| `rand` | `0.9` | Random number generation traits |
| `rand_chacha` | `0.9` | Deterministic ChaCha8 RNG for reproducible star placement |
| `wgpu` | `24.0` | Cubemap texture creation and upload |

The code lives in the `nebula-space` crate. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_star_count_within_expected_range() {
        let generator = StarfieldGenerator::new(42, 5000);
        let stars = generator.generate();
        assert_eq!(
            stars.len(),
            5000,
            "Expected exactly 5000 stars, got {}",
            stars.len()
        );
    }

    #[test]
    fn test_star_brightness_in_valid_range() {
        let generator = StarfieldGenerator::new(42, 5000);
        let stars = generator.generate();
        for (i, star) in stars.iter().enumerate() {
            assert!(
                star.brightness >= 0.0 && star.brightness <= 1.0,
                "Star {i} has brightness {} outside [0, 1]",
                star.brightness
            );
        }
    }

    #[test]
    fn test_star_directions_are_unit_vectors() {
        let generator = StarfieldGenerator::new(42, 5000);
        let stars = generator.generate();
        for (i, star) in stars.iter().enumerate() {
            let len = star.direction.length();
            assert!(
                (len - 1.0).abs() < 1e-5,
                "Star {i} direction is not a unit vector: length = {len}"
            );
        }
    }

    #[test]
    fn test_star_distribution_covers_full_sky() {
        // Partition the sky into octants (+/-x, +/-y, +/-z dominant axis)
        // and verify each octant has a reasonable number of stars.
        let generator = StarfieldGenerator::new(42, 5000);
        let stars = generator.generate();
        let mut octant_counts = [0u32; 8];

        for star in &stars {
            let d = star.direction;
            let octant = ((d.x >= 0.0) as usize)
                | (((d.y >= 0.0) as usize) << 1)
                | (((d.z >= 0.0) as usize) << 2);
            octant_counts[octant] += 1;
        }

        // With 5000 uniformly distributed stars, each octant should have ~625.
        // Allow a wide margin (300-900) to account for statistical variation.
        for (i, &count) in octant_counts.iter().enumerate() {
            assert!(
                count >= 300 && count <= 900,
                "Octant {i} has {count} stars, expected roughly 625 (range 300-900)"
            );
        }
    }

    #[test]
    fn test_same_seed_produces_same_starfield() {
        let gen_a = StarfieldGenerator::new(123, 1000);
        let gen_b = StarfieldGenerator::new(123, 1000);
        let stars_a = gen_a.generate();
        let stars_b = gen_b.generate();

        assert_eq!(stars_a.len(), stars_b.len());
        for (i, (a, b)) in stars_a.iter().zip(stars_b.iter()).enumerate() {
            assert!(
                (a.direction - b.direction).length() < 1e-6,
                "Star {i} direction differs between identical seeds"
            );
            assert!(
                (a.brightness - b.brightness).abs() < 1e-6,
                "Star {i} brightness differs between identical seeds"
            );
        }
    }

    #[test]
    fn test_different_seed_produces_different_starfield() {
        let gen_a = StarfieldGenerator::new(1, 1000);
        let gen_b = StarfieldGenerator::new(9999, 1000);
        let stars_a = gen_a.generate();
        let stars_b = gen_b.generate();

        // At least some stars should differ in direction.
        let differences = stars_a
            .iter()
            .zip(stars_b.iter())
            .filter(|(a, b)| (a.direction - b.direction).length() > 0.01)
            .count();
        assert!(
            differences > 500,
            "Expected most stars to differ between seeds, only {differences}/1000 differed"
        );
    }

    #[test]
    fn test_star_colors_are_valid_rgb() {
        let generator = StarfieldGenerator::new(42, 5000);
        let stars = generator.generate();
        for (i, star) in stars.iter().enumerate() {
            for (ch, &val) in star.color.iter().enumerate() {
                assert!(
                    val >= 0.0 && val <= 1.0,
                    "Star {i} color channel {ch} = {val} is outside [0, 1]"
                );
            }
        }
    }

    #[test]
    fn test_blackbody_red_at_low_temperature() {
        let color = blackbody_to_rgb(2000.0);
        // At 2000K, the star should be reddish: R > G > B.
        assert!(
            color[0] > color[2],
            "At 2000K, red ({}) should exceed blue ({})",
            color[0],
            color[2]
        );
    }

    #[test]
    fn test_blackbody_blue_at_high_temperature() {
        let color = blackbody_to_rgb(30000.0);
        // At 30000K, the star should be blue-white: B is high, all channels near 1.0.
        assert!(
            color[2] > 0.5,
            "At 30000K, blue channel ({}) should be high",
            color[2]
        );
    }

    #[test]
    fn test_cubemap_face_uv_mapping_covers_all_faces() {
        // A direction along each positive and negative axis should map to different faces.
        let test_dirs = [
            glam::Vec3::X,
            glam::Vec3::NEG_X,
            glam::Vec3::Y,
            glam::Vec3::NEG_Y,
            glam::Vec3::Z,
            glam::Vec3::NEG_Z,
        ];
        let mut face_indices: Vec<usize> = test_dirs
            .iter()
            .map(|d| direction_to_cube_face_uv(*d).0)
            .collect();
        face_indices.sort();
        face_indices.dedup();
        assert_eq!(
            face_indices.len(),
            6,
            "Each axis direction should map to a unique cube face"
        );
    }

    #[test]
    fn test_brightness_distribution_skews_dim() {
        // The power-law distribution should produce many more dim stars than bright ones.
        let generator = StarfieldGenerator::new(42, 5000);
        let stars = generator.generate();
        let dim_count = stars.iter().filter(|s| s.brightness < 0.1).count();
        let bright_count = stars.iter().filter(|s| s.brightness > 0.5).count();
        assert!(
            dim_count > bright_count * 3,
            "Expected many more dim stars ({dim_count}) than bright stars ({bright_count})"
        );
    }
}
```
