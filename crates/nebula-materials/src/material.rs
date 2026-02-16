//! Core material types: [`MaterialId`], [`MaterialDef`], and [`MaterialGpuData`].

use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// MaterialId
// ---------------------------------------------------------------------------

/// Compact identifier stored per voxel type to reference a material.
///
/// `MaterialId(0)` is reserved for the default material (neutral grey dielectric).
/// User-defined materials start at 1, up to a maximum of 65 535.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MaterialId(pub u16);

// ---------------------------------------------------------------------------
// MaterialError
// ---------------------------------------------------------------------------

/// Errors returned during material validation.
#[derive(Debug, Error)]
pub enum MaterialError {
    /// The material name must not be empty.
    #[error("material name must not be empty")]
    EmptyName,
}

// ---------------------------------------------------------------------------
// MaterialDef
// ---------------------------------------------------------------------------

/// Full PBR material definition for a voxel surface.
///
/// All fields are validated and clamped via [`MaterialDef::validated`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaterialDef {
    /// Human-readable name (e.g., "granite", "oak_bark", "lava").
    pub name: String,

    /// Base color in linear RGBA. Each component is clamped to `[0.0, 1.0]`.
    pub albedo: [f32; 4],

    /// Metallic factor: 0.0 = dielectric, 1.0 = pure metal. Clamped to `[0.0, 1.0]`.
    pub metallic: f32,

    /// Roughness factor: 0.0 = mirror-smooth, 1.0 = fully rough. Clamped to `[0.0, 1.0]`.
    pub roughness: f32,

    /// Emissive color in linear RGB — the color of light this surface emits.
    /// Each component is clamped to `[0.0, 1.0]`; intensity scales them.
    pub emissive_color: [f32; 3],

    /// Emissive intensity multiplier. Must be >= 0.0.
    /// 0.0 = no emission; values > 1.0 produce HDR bloom.
    pub emissive_intensity: f32,

    /// Normal map influence strength. Clamped to `[0.0, 1.0]`.
    pub normal_strength: f32,

    /// Opacity: 1.0 = fully opaque, 0.0 = fully transparent. Clamped to `[0.0, 1.0]`.
    pub opacity: f32,
}

impl Default for MaterialDef {
    fn default() -> Self {
        Self {
            name: String::from("default"),
            albedo: [0.8, 0.8, 0.8, 1.0],
            metallic: 0.0,
            roughness: 0.5,
            emissive_color: [0.0, 0.0, 0.0],
            emissive_intensity: 0.0,
            normal_strength: 1.0,
            opacity: 1.0,
        }
    }
}

impl MaterialDef {
    /// Validates and clamps all fields to their legal ranges.
    ///
    /// # Errors
    ///
    /// Returns [`MaterialError::EmptyName`] if the name is empty.
    pub fn validated(mut self) -> Result<Self, MaterialError> {
        if self.name.is_empty() {
            return Err(MaterialError::EmptyName);
        }

        for c in &mut self.albedo {
            *c = c.clamp(0.0, 1.0);
        }

        self.metallic = self.metallic.clamp(0.0, 1.0);
        self.roughness = self.roughness.clamp(0.0, 1.0);
        self.normal_strength = self.normal_strength.clamp(0.0, 1.0);
        self.opacity = self.opacity.clamp(0.0, 1.0);
        self.emissive_intensity = self.emissive_intensity.max(0.0);

        for c in &mut self.emissive_color {
            *c = c.clamp(0.0, 1.0);
        }

        Ok(self)
    }

    /// Returns `true` if this material emits light.
    pub fn is_emissive(&self) -> bool {
        self.emissive_intensity > 0.0 && self.emissive_color.iter().any(|&c| c > 0.0)
    }

    /// Total emissive contribution = color × intensity.
    pub fn emissive_output(&self) -> [f32; 3] {
        [
            self.emissive_color[0] * self.emissive_intensity,
            self.emissive_color[1] * self.emissive_intensity,
            self.emissive_color[2] * self.emissive_intensity,
        ]
    }
}

// ---------------------------------------------------------------------------
// MaterialGpuData
// ---------------------------------------------------------------------------

/// GPU-friendly packed PBR material data, 48 bytes, std140-compatible.
///
/// Suitable for upload as a storage buffer element.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct MaterialGpuData {
    /// Base color in linear RGBA.
    pub albedo: [f32; 4],
    /// xyz = emissive color, w = emissive intensity.
    pub emissive_rgb_intensity: [f32; 4],
    /// Metallic factor.
    pub metallic: f32,
    /// Roughness factor.
    pub roughness: f32,
    /// Normal map strength.
    pub normal_strength: f32,
    /// Opacity.
    pub opacity: f32,
}

impl From<MaterialDef> for MaterialGpuData {
    fn from(m: MaterialDef) -> Self {
        Self {
            albedo: m.albedo,
            emissive_rgb_intensity: [
                m.emissive_color[0],
                m.emissive_color[1],
                m.emissive_color[2],
                m.emissive_intensity,
            ],
            metallic: m.metallic,
            roughness: m.roughness,
            normal_strength: m.normal_strength,
            opacity: m.opacity,
        }
    }
}

impl From<&MaterialDef> for MaterialGpuData {
    fn from(m: &MaterialDef) -> Self {
        Self {
            albedo: m.albedo,
            emissive_rgb_intensity: [
                m.emissive_color[0],
                m.emissive_color[1],
                m.emissive_color[2],
                m.emissive_intensity,
            ],
            metallic: m.metallic,
            roughness: m.roughness,
            normal_strength: m.normal_strength,
            opacity: m.opacity,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_material_has_reasonable_values() {
        let mat = MaterialDef::default();
        assert_eq!(mat.name, "default");
        assert!(mat.albedo[0] > 0.0 && mat.albedo[0] <= 1.0);
        assert!(mat.albedo[1] > 0.0 && mat.albedo[1] <= 1.0);
        assert!(mat.albedo[2] > 0.0 && mat.albedo[2] <= 1.0);
        assert_eq!(mat.albedo[3], 1.0);
        assert_eq!(mat.metallic, 0.0);
        assert!(mat.roughness > 0.0 && mat.roughness < 1.0);
        assert_eq!(mat.emissive_intensity, 0.0);
        assert_eq!(mat.opacity, 1.0);
    }

    #[test]
    fn test_albedo_components_clamped_to_unit_range() {
        let mat = MaterialDef {
            albedo: [1.5, -0.3, 2.0, -1.0],
            ..Default::default()
        }
        .validated()
        .unwrap();

        assert_eq!(mat.albedo[0], 1.0);
        assert_eq!(mat.albedo[1], 0.0);
        assert_eq!(mat.albedo[2], 1.0);
        assert_eq!(mat.albedo[3], 0.0);
    }

    #[test]
    fn test_metallic_clamped_to_unit_range() {
        let too_high = MaterialDef {
            metallic: 1.5,
            ..Default::default()
        }
        .validated()
        .unwrap();
        assert_eq!(too_high.metallic, 1.0);

        let too_low = MaterialDef {
            metallic: -0.2,
            ..Default::default()
        }
        .validated()
        .unwrap();
        assert_eq!(too_low.metallic, 0.0);

        let valid = MaterialDef {
            metallic: 0.7,
            ..Default::default()
        }
        .validated()
        .unwrap();
        assert_eq!(valid.metallic, 0.7);
    }

    #[test]
    fn test_roughness_clamped_to_unit_range() {
        let too_high = MaterialDef {
            roughness: 2.0,
            ..Default::default()
        }
        .validated()
        .unwrap();
        assert_eq!(too_high.roughness, 1.0);

        let too_low = MaterialDef {
            roughness: -0.5,
            ..Default::default()
        }
        .validated()
        .unwrap();
        assert_eq!(too_low.roughness, 0.0);
    }

    #[test]
    fn test_emissive_intensity_non_negative() {
        let negative = MaterialDef {
            emissive_intensity: -5.0,
            ..Default::default()
        }
        .validated()
        .unwrap();
        assert_eq!(negative.emissive_intensity, 0.0);

        let hdr = MaterialDef {
            emissive_intensity: 10.0,
            ..Default::default()
        }
        .validated()
        .unwrap();
        assert_eq!(hdr.emissive_intensity, 10.0);
    }

    #[test]
    fn test_material_id_is_unique() {
        let id_a = MaterialId(0);
        let id_b = MaterialId(1);
        assert_ne!(id_a, id_b);

        let id_c = MaterialId(42);
        let id_d = MaterialId(42);
        assert_eq!(id_c, id_d);
    }

    #[test]
    fn test_material_gpu_data_size_and_alignment() {
        assert_eq!(std::mem::size_of::<MaterialGpuData>(), 48);
        assert!(std::mem::align_of::<MaterialGpuData>() >= 4);
    }

    #[test]
    fn test_material_def_to_gpu_data_conversion() {
        let mat = MaterialDef {
            albedo: [1.0, 0.0, 0.5, 1.0],
            metallic: 0.9,
            roughness: 0.1,
            emissive_color: [1.0, 0.5, 0.0],
            emissive_intensity: 3.0,
            normal_strength: 0.8,
            opacity: 1.0,
            ..Default::default()
        };
        let gpu: MaterialGpuData = mat.into();
        assert_eq!(gpu.albedo, [1.0, 0.0, 0.5, 1.0]);
        assert_eq!(gpu.metallic, 0.9);
        assert_eq!(gpu.roughness, 0.1);
        assert_eq!(gpu.emissive_rgb_intensity, [1.0, 0.5, 0.0, 3.0]);
        assert_eq!(gpu.normal_strength, 0.8);
        assert_eq!(gpu.opacity, 1.0);
    }

    #[test]
    fn test_empty_name_rejected() {
        let result = MaterialDef {
            name: String::new(),
            ..Default::default()
        }
        .validated();
        assert!(result.is_err());
    }

    #[test]
    fn test_is_emissive() {
        let non_emissive = MaterialDef::default();
        assert!(!non_emissive.is_emissive());

        let emissive = MaterialDef {
            emissive_color: [1.0, 0.0, 0.0],
            emissive_intensity: 2.0,
            ..Default::default()
        };
        assert!(emissive.is_emissive());
    }

    #[test]
    fn test_emissive_output() {
        let mat = MaterialDef {
            emissive_color: [1.0, 0.5, 0.0],
            emissive_intensity: 3.0,
            ..Default::default()
        };
        let out = mat.emissive_output();
        assert!((out[0] - 3.0).abs() < 1e-6);
        assert!((out[1] - 1.5).abs() < 1e-6);
        assert!((out[2] - 0.0).abs() < 1e-6);
    }
}
