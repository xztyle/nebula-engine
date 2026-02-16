//! Material blending: per-vertex blend weight computation and triplanar projection helpers.
//!
//! Biome boundaries produce smooth transitions via [`compute_blend_weight`], while
//! [`triplanar_weights`] mirrors the WGSL triplanar projection logic for CPU-side use.

use glam::Vec3;

use crate::MaterialId;

/// A biome map trait that provides primary/secondary biome blending data.
///
/// Implementors supply biome sampling for a given world column (x, z).
pub trait BiomeMap {
    /// Sample the biome at the given world-space column.
    ///
    /// Returns `(primary_material, secondary_material, blend_factor)` where
    /// `blend_factor` is in \[0.0, 1.0\]. A value of 0.0 means fully primary,
    /// 1.0 means fully secondary.
    fn sample(&self, world_x: i128, world_z: i128) -> (MaterialId, MaterialId, f32);
}

/// Compute the blend weight for a voxel at the given position.
///
/// Returns `(material_a, material_b, blend_factor)` from the biome map.
/// When not at a boundary, `material_b == material_a` and `blend_factor == 0.0`.
pub fn compute_blend_weight(
    biome_map: &dyn BiomeMap,
    world_x: i128,
    world_z: i128,
) -> (MaterialId, MaterialId, f32) {
    biome_map.sample(world_x, world_z)
}

/// Compute triplanar projection blend weights from a surface normal.
///
/// Mirrors the WGSL `triplanar_sample` weight calculation: absolute normal
/// components raised to the 4th power, then normalized to sum to 1.0.
pub fn triplanar_weights(normal: Vec3) -> Vec3 {
    let mut blend = normal.abs();
    blend = Vec3::new(blend.x.powi(4), blend.y.powi(4), blend.z.powi(4));
    let sum = blend.x + blend.y + blend.z;
    blend / sum
}

/// Linear blend between two RGB colors.
///
/// `w = 0.0` returns `a`, `w = 1.0` returns `b`.
pub fn blend_colors(a: [f32; 3], b: [f32; 3], w: f32) -> [f32; 3] {
    [
        a[0] * (1.0 - w) + b[0] * w,
        a[1] * (1.0 - w) + b[1] * w,
        a[2] * (1.0 - w) + b[2] * w,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a test color for a material (simulating atlas sampling).
    fn color_a() -> [f32; 3] {
        [1.0, 0.0, 0.0]
    } // red
    fn color_b() -> [f32; 3] {
        [0.0, 0.0, 1.0]
    } // blue

    #[test]
    fn test_blend_weight_zero_shows_material_a_only() {
        let result = blend_colors(color_a(), color_b(), 0.0);
        assert_eq!(result, [1.0, 0.0, 0.0]); // pure red (material A)
    }

    #[test]
    fn test_blend_weight_one_shows_material_b_only() {
        let result = blend_colors(color_a(), color_b(), 1.0);
        assert_eq!(result, [0.0, 0.0, 1.0]); // pure blue (material B)
    }

    #[test]
    fn test_blend_weight_half_blends_equally() {
        let result = blend_colors(color_a(), color_b(), 0.5);
        // 50% red + 50% blue = (0.5, 0.0, 0.5)
        let epsilon = 1e-6;
        assert!((result[0] - 0.5).abs() < epsilon);
        assert!((result[1] - 0.0).abs() < epsilon);
        assert!((result[2] - 0.5).abs() < epsilon);
    }

    #[test]
    fn test_triplanar_projection_eliminates_stretching_on_vertical_faces() {
        // For a vertical face with normal (1, 0, 0) — pointing along +X —
        // triplanar blending weights should be (1, 0, 0), meaning only
        // the YZ-projected texture is used (no stretching).
        let normal = Vec3::new(1.0, 0.0, 0.0);
        let blend = triplanar_weights(normal);

        assert!(
            blend.x > 0.99,
            "X-facing surface should use X projection: {blend:?}"
        );
        assert!(
            blend.y < 0.01,
            "Y projection weight should be ~0 for X-facing surface"
        );
        assert!(
            blend.z < 0.01,
            "Z projection weight should be ~0 for X-facing surface"
        );
    }

    #[test]
    fn test_triplanar_weights_for_horizontal_face() {
        // For a horizontal face with normal (0, 1, 0) — pointing up —
        // only the XZ-projected texture should be used.
        let normal = Vec3::new(0.0, 1.0, 0.0);
        let blend = triplanar_weights(normal);

        assert!(
            blend.y > 0.99,
            "Y-facing surface should use Y projection: {blend:?}"
        );
        assert!(blend.x < 0.01);
        assert!(blend.z < 0.01);
    }

    #[test]
    fn test_triplanar_weights_for_diagonal_face() {
        // A 45-degree surface with normal (0.707, 0.707, 0) should blend
        // X and Y projections roughly equally, with Z near zero.
        let normal = Vec3::new(0.707, 0.707, 0.0).normalize();
        let blend = triplanar_weights(normal);

        let epsilon = 0.05;
        assert!(
            (blend.x - blend.y).abs() < epsilon,
            "Diagonal surface should blend X and Y equally: {blend:?}"
        );
        assert!(
            blend.z < 0.01,
            "Z weight should be ~0 for XY-diagonal surface: {blend:?}"
        );
    }

    #[test]
    fn test_triplanar_weights_sum_to_one() {
        let normals = [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.577, 0.577, 0.577), // diagonal
            Vec3::new(0.2, 0.8, 0.1).normalize(),
        ];

        for normal in normals {
            let blend = triplanar_weights(normal);
            let sum = blend.x + blend.y + blend.z;
            assert!(
                (sum - 1.0).abs() < 1e-4,
                "Triplanar weights should sum to 1.0, got {sum} for normal {normal:?}"
            );
        }
    }

    #[test]
    fn test_blending_is_smooth_no_hard_edges() {
        // Verify that small changes in blend weight produce small changes in output.
        let steps = 100;
        let mut prev_result = blend_colors(color_a(), color_b(), 0.0);

        for i in 1..=steps {
            let w = i as f32 / steps as f32;
            let result = blend_colors(color_a(), color_b(), w);

            // The maximum change per step should be bounded
            let max_delta = 1.0 / steps as f32 + 1e-6;
            for c in 0..3 {
                let delta = (result[c] - prev_result[c]).abs();
                assert!(
                    delta <= max_delta + 1e-6,
                    "Discontinuity at w={w}: channel {c} changed by {delta} (max {max_delta})"
                );
            }

            prev_result = result;
        }
    }
}
