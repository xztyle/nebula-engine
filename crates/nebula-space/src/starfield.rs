//! Procedural starfield generation: deterministic star placement on the sky sphere,
//! baked into a cubemap texture for efficient skybox rendering.

use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

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

/// Generates a deterministic catalog of stars from a universe seed.
pub struct StarfieldGenerator {
    seed: u64,
    star_count: u32,
}

impl StarfieldGenerator {
    /// Create a new generator with the given seed and star count.
    pub fn new(seed: u64, star_count: u32) -> Self {
        Self { seed, star_count }
    }

    /// Generate the star catalog. Deterministic for a given seed.
    pub fn generate(&self) -> Vec<StarPoint> {
        let mut rng = ChaCha8Rng::seed_from_u64(self.seed);
        let mut stars = Vec::with_capacity(self.star_count as usize);

        for _ in 0..self.star_count {
            let theta = rng.random::<f32>() * std::f32::consts::TAU;
            let phi = (1.0 - 2.0 * rng.random::<f32>()).acos();

            let direction =
                glam::Vec3::new(phi.sin() * theta.cos(), phi.sin() * theta.sin(), phi.cos());

            // Power-law brightness: many dim stars, few bright ones.
            let raw: f32 = rng.random();
            // Power-law: many dim, few bright.
            let brightness = raw.powf(4.0).clamp(0.0, 1.0);

            let temperature = 2000.0 + brightness * 28000.0;
            let color = blackbody_to_rgb(temperature);

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

/// Convert a blackbody temperature in Kelvin to an approximate sRGB color.
///
/// Uses a simplified Planckian locus approximation (Tanner Helland algorithm).
pub fn blackbody_to_rgb(temperature_k: f32) -> [f32; 3] {
    let t = temperature_k / 100.0;
    let r = if t <= 66.0 {
        1.0
    } else {
        (329.698_73 * (t - 60.0).powf(-0.133_204_76) / 255.0).clamp(0.0, 1.0)
    };
    let g = if t <= 66.0 {
        (99.470_8 * t.ln() - 161.119_57).clamp(0.0, 255.0) / 255.0
    } else {
        (288.122_17 * (t - 60.0).powf(-0.075_514_85) / 255.0).clamp(0.0, 1.0)
    };
    let b = if t >= 66.0 {
        1.0
    } else if t <= 19.0 {
        0.0
    } else {
        (138.517_73 * (t - 10.0).ln() - 305.044_8).clamp(0.0, 255.0) / 255.0
    };
    [r, g, b]
}

/// A cubemap texture containing rendered star pixels.
pub struct StarfieldCubemap {
    /// Width/height of each cubemap face in pixels.
    pub face_size: u32,
    /// Six faces, each `face_size * face_size` pixels, stored as RGBA f32.
    pub faces: [Vec<[f32; 4]>; 6],
}

impl StarfieldCubemap {
    /// Render a star catalog into a cubemap texture.
    pub fn render(stars: &[StarPoint], face_size: u32) -> Self {
        let pixel_count = (face_size * face_size) as usize;
        let mut faces: [Vec<[f32; 4]>; 6] =
            std::array::from_fn(|_| vec![[0.0, 0.0, 0.0, 1.0]; pixel_count]);

        for star in stars {
            let (face_index, u, v) = direction_to_cube_face_uv(star.direction);

            let px = (u * face_size as f32).min(face_size as f32 - 1.0) as u32;
            let py = (v * face_size as f32).min(face_size as f32 - 1.0) as u32;
            let idx = (py * face_size + px) as usize;

            // Additive blend: multiple dim stars in same pixel accumulate.
            let pixel = &mut faces[face_index][idx];
            // Boost brightness: ensure even dim stars produce visible pixels.
            let b = star.brightness * 8.0 + 0.4;
            pixel[0] = (pixel[0] + star.color[0] * b).min(1.0);
            pixel[1] = (pixel[1] + star.color[1] * b).min(1.0);
            pixel[2] = (pixel[2] + star.color[2] * b).min(1.0);

            // Bright stars bleed into neighboring pixels for a glow effect.
            if star.brightness > 0.3 {
                let glow = star.brightness * 2.0;
                let offsets: [(i32, i32); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];
                for (dx, dy) in offsets {
                    let nx = px as i32 + dx;
                    let ny = py as i32 + dy;
                    if nx >= 0 && nx < face_size as i32 && ny >= 0 && ny < face_size as i32 {
                        let ni = (ny as u32 * face_size + nx as u32) as usize;
                        let np = &mut faces[face_index][ni];
                        np[0] = (np[0] + star.color[0] * glow * 0.3).min(1.0);
                        np[1] = (np[1] + star.color[1] * glow * 0.3).min(1.0);
                        np[2] = (np[2] + star.color[2] * glow * 0.3).min(1.0);
                    }
                }
            }
        }

        Self { face_size, faces }
    }

    /// Convert face data to RGBA8 bytes suitable for GPU upload.
    ///
    /// Returns a `Vec` of 6 face byte arrays, each `face_size * face_size * 4` bytes.
    pub fn to_rgba8(&self) -> Vec<Vec<u8>> {
        self.faces
            .iter()
            .map(|face| {
                let mut bytes = Vec::with_capacity(face.len() * 4);
                for pixel in face {
                    bytes.push((pixel[0].clamp(0.0, 1.0) * 255.0) as u8);
                    bytes.push((pixel[1].clamp(0.0, 1.0) * 255.0) as u8);
                    bytes.push((pixel[2].clamp(0.0, 1.0) * 255.0) as u8);
                    bytes.push((pixel[3].clamp(0.0, 1.0) * 255.0) as u8);
                }
                bytes
            })
            .collect()
    }
}

/// Map a unit direction vector to a cube face index (0..6) and UV coordinates in [0, 1].
///
/// Face indices: 0=+X, 1=-X, 2=+Y, 3=-Y, 4=+Z, 5=-Z.
pub(crate) fn direction_to_cube_face_uv(dir: glam::Vec3) -> (usize, f32, f32) {
    let abs = dir.abs();
    let (face, u, v) = if abs.x >= abs.y && abs.x >= abs.z {
        if dir.x > 0.0 {
            (0, -dir.z / abs.x, -dir.y / abs.x) // +X
        } else {
            (1, dir.z / abs.x, -dir.y / abs.x) // -X
        }
    } else if abs.y >= abs.x && abs.y >= abs.z {
        if dir.y > 0.0 {
            (2, dir.x / abs.y, dir.z / abs.y) // +Y
        } else {
            (3, dir.x / abs.y, -dir.z / abs.y) // -Y
        }
    } else if dir.z > 0.0 {
        (4, dir.x / abs.z, -dir.y / abs.z) // +Z
    } else {
        (5, -dir.x / abs.z, -dir.y / abs.z) // -Z
    };
    // Remap from [-1, 1] to [0, 1].
    (face, u * 0.5 + 0.5, v * 0.5 + 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_star_count_within_expected_range() {
        let generator = StarfieldGenerator::new(42, 5000);
        let stars = generator.generate();
        assert_eq!(stars.len(), 5000);
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

        for (i, &count) in octant_counts.iter().enumerate() {
            assert!(
                (300..=900).contains(&count),
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
                    (0.0..=1.0).contains(&val),
                    "Star {i} color channel {ch} = {val} is outside [0, 1]"
                );
            }
        }
    }

    #[test]
    fn test_blackbody_red_at_low_temperature() {
        let color = blackbody_to_rgb(2000.0);
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
        assert!(
            color[2] > 0.5,
            "At 30000K, blue channel ({}) should be high",
            color[2]
        );
    }

    #[test]
    fn test_cubemap_face_uv_mapping_covers_all_faces() {
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
        let generator = StarfieldGenerator::new(42, 5000);
        let stars = generator.generate();
        let dim_count = stars.iter().filter(|s| s.brightness < 0.1).count();
        let bright_count = stars.iter().filter(|s| s.brightness > 0.5).count();
        assert!(
            dim_count > bright_count * 3,
            "Expected many more dim stars ({dim_count}) than bright stars ({bright_count})"
        );
    }

    #[test]
    fn test_cubemap_render_produces_non_empty_faces() {
        let generator = StarfieldGenerator::new(42, 5000);
        let stars = generator.generate();
        let cubemap = StarfieldCubemap::render(&stars, 128);

        // At least some faces should have non-zero pixels
        let mut total_lit = 0usize;
        for face in &cubemap.faces {
            for pixel in face {
                if pixel[0] > 0.0 || pixel[1] > 0.0 || pixel[2] > 0.0 {
                    total_lit += 1;
                }
            }
        }
        assert!(
            total_lit > 100,
            "Expected many lit pixels in cubemap, got {total_lit}"
        );
    }

    #[test]
    fn test_cubemap_to_rgba8() {
        let generator = StarfieldGenerator::new(42, 100);
        let stars = generator.generate();
        let cubemap = StarfieldCubemap::render(&stars, 16);
        let rgba8 = cubemap.to_rgba8();
        assert_eq!(rgba8.len(), 6);
        for face_bytes in &rgba8 {
            assert_eq!(face_bytes.len(), 16 * 16 * 4);
        }
    }
}
