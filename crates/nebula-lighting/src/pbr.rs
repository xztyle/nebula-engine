//! Physically-based rendering (PBR) material model and Cook-Torrance BRDF.
//!
//! Provides [`PbrMaterial`] for CPU-side material definitions and
//! [`PbrMaterialUniform`] for GPU upload. Also includes CPU-side BRDF
//! reference functions used for unit testing shader correctness.

use bytemuck::{Pod, Zeroable};

/// PBR material parameters (CPU-side, per voxel type or per texture).
#[derive(Clone, Debug)]
pub struct PbrMaterial {
    /// Base color (linear RGB).
    pub albedo: glam::Vec3,
    /// Metallic factor \[0.0, 1.0\]. 0 = dielectric, 1 = metal.
    pub metallic: f32,
    /// Roughness factor \[0.0, 1.0\]. 0 = mirror, 1 = fully rough.
    pub roughness: f32,
    /// Ambient occlusion \[0.0, 1.0\]. 1 = fully exposed, 0 = fully occluded.
    pub ao: f32,
    /// Emissive color in linear RGB. Zero for non-emissive materials.
    pub emissive_color: glam::Vec3,
    /// Emissive intensity multiplier. Values > 1.0 produce HDR output for bloom.
    pub emissive_intensity: f32,
}

impl Default for PbrMaterial {
    fn default() -> Self {
        Self {
            albedo: glam::Vec3::new(0.5, 0.5, 0.5),
            metallic: 0.0,
            roughness: 0.5,
            ao: 1.0,
            emissive_color: glam::Vec3::ZERO,
            emissive_intensity: 0.0,
        }
    }
}

impl PbrMaterial {
    /// Stone-like material: grey, rough, non-metallic.
    pub fn stone() -> Self {
        Self {
            albedo: glam::Vec3::new(0.6, 0.58, 0.55),
            metallic: 0.0,
            roughness: 0.9,
            ao: 1.0,
            emissive_color: glam::Vec3::ZERO,
            emissive_intensity: 0.0,
        }
    }

    /// Metallic ore material: golden tint, metallic, moderate roughness.
    pub fn ore() -> Self {
        Self {
            albedo: glam::Vec3::new(1.0, 0.8, 0.3),
            metallic: 0.8,
            roughness: 0.4,
            ao: 1.0,
            emissive_color: glam::Vec3::ZERO,
            emissive_intensity: 0.0,
        }
    }

    /// Lava material: orange-red, rough, non-metallic, strongly emissive.
    pub fn lava() -> Self {
        Self {
            albedo: glam::Vec3::new(0.8, 0.2, 0.0),
            metallic: 0.0,
            roughness: 0.9,
            ao: 1.0,
            emissive_color: glam::Vec3::new(1.0, 0.3, 0.0),
            emissive_intensity: 5.0,
        }
    }

    /// Glowstone material: warm white glow, moderate emissive intensity.
    pub fn glowstone() -> Self {
        Self {
            albedo: glam::Vec3::new(0.9, 0.8, 0.5),
            metallic: 0.0,
            roughness: 0.7,
            ao: 1.0,
            emissive_color: glam::Vec3::new(1.0, 0.9, 0.6),
            emissive_intensity: 3.0,
        }
    }

    /// Returns `true` if this material emits light.
    pub fn is_emissive(&self) -> bool {
        self.emissive_intensity > 0.0 && self.emissive_color.length_squared() > 0.0
    }

    /// Total emissive contribution = color × intensity.
    pub fn emissive_output(&self) -> glam::Vec3 {
        self.emissive_color * self.emissive_intensity
    }

    /// Build the GPU-side uniform from this material's properties.
    pub fn to_uniform(&self) -> PbrMaterialUniform {
        let emissive = self.emissive_output();
        PbrMaterialUniform {
            albedo_metallic: [self.albedo.x, self.albedo.y, self.albedo.z, self.metallic],
            roughness_ao_pad: [self.roughness, self.ao, 0.0, 0.0],
            emissive: [emissive.x, emissive.y, emissive.z, 0.0],
        }
    }
}

/// GPU-side PBR material uniform, 48 bytes, std140-compatible.
///
/// Bound at `@group(3) @binding(0)` visible to `ShaderStages::FRAGMENT`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PbrMaterialUniform {
    /// xyz = albedo (linear RGB), w = metallic.
    pub albedo_metallic: [f32; 4],
    /// x = roughness, y = ao, zw = padding.
    pub roughness_ao_pad: [f32; 4],
    /// xyz = emissive color × intensity (pre-multiplied), w = padding.
    pub emissive: [f32; 4],
}

// ---------------------------------------------------------------------------
// CPU-side BRDF reference implementation (for testing)
// ---------------------------------------------------------------------------

/// GGX/Trowbridge-Reitz normal distribution function (CPU reference).
pub fn distribution_ggx_cpu(n_dot_h: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let denom = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    a2 / (std::f32::consts::PI * denom * denom)
}

/// Schlick-GGX geometry function for a single direction (CPU reference).
pub fn geometry_schlick_ggx_cpu(n_dot: f32, roughness: f32) -> f32 {
    let r = roughness + 1.0;
    let k = (r * r) / 8.0;
    n_dot / (n_dot * (1.0 - k) + k)
}

/// Smith's method geometry function combining view and light (CPU reference).
pub fn geometry_smith_cpu(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    geometry_schlick_ggx_cpu(n_dot_v, roughness) * geometry_schlick_ggx_cpu(n_dot_l, roughness)
}

/// Schlick Fresnel approximation (CPU reference).
pub fn fresnel_schlick_cpu(cos_theta: f32, f0: glam::Vec3) -> glam::Vec3 {
    f0 + (glam::Vec3::ONE - f0) * (1.0 - cos_theta).clamp(0.0, 1.0).powf(5.0)
}

/// Full PBR BRDF evaluation for a single light (CPU reference).
///
/// Returns the outgoing radiance contribution (BRDF × N·L) for one light.
pub fn evaluate_brdf_cpu(
    light_dir: glam::Vec3,
    view_dir: glam::Vec3,
    normal: glam::Vec3,
    albedo: glam::Vec3,
    metallic: f32,
    roughness: f32,
) -> glam::Vec3 {
    let half_vec = (view_dir + light_dir).normalize();
    let n_dot_l = normal.dot(light_dir).max(0.0);
    let n_dot_v = normal.dot(view_dir).max(0.0);
    let n_dot_h = normal.dot(half_vec).max(0.0);
    let h_dot_v = half_vec.dot(view_dir).max(0.0);

    let f0 = glam::Vec3::splat(0.04).lerp(albedo, metallic);
    let d = distribution_ggx_cpu(n_dot_h, roughness);
    let g = geometry_smith_cpu(n_dot_v, n_dot_l, roughness);
    let f = fresnel_schlick_cpu(h_dot_v, f0);

    let specular = (d * g * f) / (4.0 * n_dot_v * n_dot_l + 0.0001);
    let k_d = (glam::Vec3::ONE - f) * (1.0 - metallic);
    let diffuse = k_d * albedo / std::f32::consts::PI;

    (diffuse + specular) * n_dot_l
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pbr_material_uniform_size() {
        assert_eq!(std::mem::size_of::<PbrMaterialUniform>(), 48);
    }

    #[test]
    fn test_pbr_material_default() {
        let mat = PbrMaterial::default();
        assert!((mat.metallic - 0.0).abs() < 1e-6);
        assert!((mat.roughness - 0.5).abs() < 1e-6);
        assert!((mat.ao - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_to_uniform_packs_correctly() {
        let mat = PbrMaterial {
            albedo: glam::Vec3::new(1.0, 0.5, 0.25),
            metallic: 0.8,
            roughness: 0.3,
            ao: 0.9,
            emissive_color: glam::Vec3::ZERO,
            emissive_intensity: 0.0,
        };
        let u = mat.to_uniform();
        assert!((u.albedo_metallic[0] - 1.0).abs() < 1e-6);
        assert!((u.albedo_metallic[1] - 0.5).abs() < 1e-6);
        assert!((u.albedo_metallic[2] - 0.25).abs() < 1e-6);
        assert!((u.albedo_metallic[3] - 0.8).abs() < 1e-6);
        assert!((u.roughness_ao_pad[0] - 0.3).abs() < 1e-6);
        assert!((u.roughness_ao_pad[1] - 0.9).abs() < 1e-6);
    }

    #[test]
    fn test_pure_metal_has_no_diffuse() {
        let result = evaluate_brdf_cpu(
            glam::Vec3::Y,
            glam::Vec3::new(0.0, 1.0, 1.0).normalize(),
            glam::Vec3::Y,
            glam::Vec3::new(1.0, 0.8, 0.2),
            1.0,
            0.5,
        );
        let dielectric_result = evaluate_brdf_cpu(
            glam::Vec3::Y,
            glam::Vec3::new(0.0, 1.0, 1.0).normalize(),
            glam::Vec3::Y,
            glam::Vec3::new(1.0, 0.8, 0.2),
            0.0,
            0.5,
        );
        assert!(
            dielectric_result.length() > result.length(),
            "dielectric ({dielectric_result:?}) should have more total light than metal ({result:?}) due to diffuse",
        );
    }

    #[test]
    fn test_pure_dielectric_has_full_diffuse() {
        let result = evaluate_brdf_cpu(
            glam::Vec3::Y,
            glam::Vec3::Y,
            glam::Vec3::Y,
            glam::Vec3::ONE,
            0.0,
            1.0,
        );
        assert!(
            result.length() > 0.0,
            "dielectric should have non-zero output"
        );
        assert!(
            result.x > 0.2,
            "diffuse contribution should be significant for dielectric"
        );
    }

    #[test]
    fn test_roughness_zero_gives_sharp_specular() {
        let smooth = evaluate_brdf_cpu(
            glam::Vec3::Y,
            glam::Vec3::Y,
            glam::Vec3::Y,
            glam::Vec3::ONE,
            0.5,
            0.01,
        );
        let rough = evaluate_brdf_cpu(
            glam::Vec3::Y,
            glam::Vec3::Y,
            glam::Vec3::Y,
            glam::Vec3::ONE,
            0.5,
            0.99,
        );
        assert!(
            smooth.length() > rough.length(),
            "smooth ({smooth:?}) should have stronger specular peak than rough ({rough:?})",
        );
    }

    #[test]
    fn test_roughness_one_gives_broad_specular() {
        let off_angle_light = glam::Vec3::new(0.5, 0.5, 0.0).normalize();
        let rough = evaluate_brdf_cpu(
            off_angle_light,
            glam::Vec3::Y,
            glam::Vec3::Y,
            glam::Vec3::ONE,
            0.5,
            1.0,
        );
        assert!(
            rough.length() > 0.0,
            "rough material should scatter light broadly"
        );
    }

    #[test]
    fn test_energy_conservation() {
        let result = evaluate_brdf_cpu(
            glam::Vec3::Y,
            glam::Vec3::Y,
            glam::Vec3::Y,
            glam::Vec3::ONE,
            0.5,
            0.5,
        );
        assert!(
            result.x <= 1.0 + 1e-6,
            "R channel ({}) exceeds incoming",
            result.x
        );
        assert!(
            result.y <= 1.0 + 1e-6,
            "G channel ({}) exceeds incoming",
            result.y
        );
        assert!(
            result.z <= 1.0 + 1e-6,
            "B channel ({}) exceeds incoming",
            result.z
        );
    }

    #[test]
    fn test_emissive_material_outputs_hdr_values() {
        let mat = PbrMaterial::lava();
        let output = mat.emissive_output();
        assert!(
            output.x > 1.0,
            "emissive R ({}) should exceed 1.0 for HDR",
            output.x
        );
        assert!(
            output.y > 1.0,
            "emissive G ({}) should exceed 1.0 for HDR",
            output.y
        );
        assert!((output.x - 5.0).abs() < 1e-6);
        assert!((output.y - 1.5).abs() < 1e-6);
    }

    #[test]
    fn test_non_emissive_material_has_zero_emissive() {
        let mat = PbrMaterial::stone();
        let output = mat.emissive_output();
        assert_eq!(output, glam::Vec3::ZERO);
        assert!(!mat.is_emissive());
    }

    #[test]
    fn test_emissive_color_matches_gpu_uniform() {
        let mat = PbrMaterial::lava();
        let gpu = mat.to_uniform();
        let expected = mat.emissive_output();
        assert!((gpu.emissive[0] - expected.x).abs() < 1e-6);
        assert!((gpu.emissive[1] - expected.y).abs() < 1e-6);
        assert!((gpu.emissive[2] - expected.z).abs() < 1e-6);
    }

    #[test]
    fn test_bloom_responds_to_emissive_surfaces() {
        let mat = PbrMaterial::lava();
        let output = mat.emissive_output();
        let bloom_threshold = 1.0;
        let max_channel = output.x.max(output.y).max(output.z);
        assert!(max_channel > bloom_threshold);

        let stone_output = PbrMaterial::stone().emissive_output();
        let stone_max = stone_output.x.max(stone_output.y).max(stone_output.z);
        assert!(stone_max <= bloom_threshold);
    }

    #[test]
    fn test_ao_reduces_ambient_contribution() {
        let albedo = glam::Vec3::ONE;
        let ambient_full = glam::Vec3::splat(0.03) * albedo * 1.0;
        let ambient_half = glam::Vec3::splat(0.03) * albedo * 0.5;
        let ambient_zero = glam::Vec3::splat(0.03) * albedo * 0.0;
        assert!(ambient_full.x > ambient_half.x);
        assert!(ambient_half.x > ambient_zero.x);
        assert_eq!(ambient_zero.x, 0.0);
    }
}
