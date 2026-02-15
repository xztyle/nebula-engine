# Material Properties

## Problem

Every voxel in the engine references a material that describes its visual appearance for PBR rendering — albedo color, metallic/roughness factors, emissive glow, normal perturbation, and opacity. Without a structured material definition, these properties would be scattered across ad-hoc fields in the voxel type registry or hard-coded in shaders, making it impossible to add new materials without code changes. The engine needs a single, compact, validated struct that fully describes a material's surface properties, assigned a unique `MaterialId` so that voxel types, meshing, and the render pipeline can reference materials by a cheap numeric handle. Materials must be immutable once registered to guarantee thread-safe sharing across ECS systems and to allow the GPU uniform buffer to be written once at startup.

## Solution

Introduce a `MaterialDef` struct and a `MaterialId` newtype in the `nebula_materials` crate. Each material is registered at startup and receives a sequential `MaterialId`. Once registration is complete, no further mutations are permitted.

### Data Structures

```rust
/// Compact identifier stored per voxel type to reference a material.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct MaterialId(pub u16);

/// Full PBR material definition for a voxel surface.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MaterialDef {
    /// Human-readable name (e.g., "granite", "oak_bark", "lava").
    pub name: String,

    /// Base color in linear RGBA. Each component must be in [0.0, 1.0].
    pub albedo: [f32; 4],

    /// Metallic factor: 0.0 = dielectric (stone, wood), 1.0 = pure metal (iron, gold).
    /// Clamped to [0.0, 1.0] on construction.
    pub metallic: f32,

    /// Roughness factor: 0.0 = mirror-smooth, 1.0 = fully rough.
    /// Clamped to [0.0, 1.0] on construction.
    pub roughness: f32,

    /// Emissive color in linear RGB — the color of light this surface emits.
    pub emissive_color: [f32; 3],

    /// Emissive intensity multiplier. Must be >= 0.0.
    /// 0.0 = no emission; values > 1.0 produce HDR bloom.
    pub emissive_intensity: f32,

    /// Normal map influence strength: 0.0 = flat (ignore normal map),
    /// 1.0 = full normal map effect. Clamped to [0.0, 1.0].
    pub normal_strength: f32,

    /// Opacity: 1.0 = fully opaque, 0.0 = fully transparent.
    /// Clamped to [0.0, 1.0].
    pub opacity: f32,
}
```

### Validation

All fields are validated at construction time through a builder or a `validate()` method:

```rust
impl MaterialDef {
    /// Validates and clamps all fields to their legal ranges.
    /// Returns an error if the name is empty.
    pub fn validated(mut self) -> Result<Self, MaterialError> {
        if self.name.is_empty() {
            return Err(MaterialError::EmptyName);
        }

        // Clamp albedo components to [0, 1]
        for c in &mut self.albedo {
            *c = c.clamp(0.0, 1.0);
        }

        self.metallic = self.metallic.clamp(0.0, 1.0);
        self.roughness = self.roughness.clamp(0.0, 1.0);
        self.normal_strength = self.normal_strength.clamp(0.0, 1.0);
        self.opacity = self.opacity.clamp(0.0, 1.0);
        self.emissive_intensity = self.emissive_intensity.max(0.0);

        // Clamp emissive color components to [0, 1] (intensity scales them)
        for c in &mut self.emissive_color {
            *c = c.clamp(0.0, 1.0);
        }

        Ok(self)
    }
}
```

### Default Material

The default material represents a neutral, fully opaque, non-emissive, medium-rough dielectric:

```rust
impl Default for MaterialDef {
    fn default() -> Self {
        Self {
            name: String::from("default"),
            albedo: [0.8, 0.8, 0.8, 1.0],   // light grey, fully opaque
            metallic: 0.0,                     // dielectric
            roughness: 0.5,                    // medium rough
            emissive_color: [0.0, 0.0, 0.0],  // no emission
            emissive_intensity: 0.0,
            normal_strength: 1.0,              // full normal map effect
            opacity: 1.0,                      // fully opaque
        }
    }
}
```

### MaterialId Assignment

`MaterialId(0)` is reserved for the default material (used as a fallback for missing or invalid materials). User-defined materials are assigned IDs starting from 1, incrementing sequentially. The maximum capacity is 65,535 user-defined materials (u16 range minus the reserved slot). This keeps the per-voxel-type material reference at exactly 2 bytes.

### GPU Representation

For upload to the GPU as a uniform/storage buffer, the material data is packed into a `MaterialGpuData` struct aligned to 16-byte boundaries:

```rust
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MaterialGpuData {
    pub albedo: [f32; 4],           // 16 bytes
    pub emissive_rgb_intensity: [f32; 4], // rgb + intensity, 16 bytes
    pub metallic: f32,              // 4 bytes
    pub roughness: f32,             // 4 bytes
    pub normal_strength: f32,       // 4 bytes
    pub opacity: f32,               // 4 bytes
}
// Total: 48 bytes per material, aligned to 16 bytes
```

## Outcome

A `MaterialDef` struct and `MaterialId` newtype in `nebula_materials` that fully describe a voxel surface's PBR properties. All fields are validated and clamped on construction. A default material is always available at `MaterialId(0)`. The `MaterialGpuData` struct provides a GPU-friendly packed representation suitable for upload as a storage buffer. Running `cargo test -p nebula_materials` passes all material property tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Each voxel type now has PBR material properties: albedo color, roughness, metallic. Stone is grey and rough; grass is green and smooth. The visual quality jumps dramatically.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `serde` | `1.0` with `derive` | Serialize/deserialize `MaterialDef` for RON asset loading |
| `bytemuck` | `1.21` | Pod/Zeroable derives for `MaterialGpuData` GPU upload |
| `thiserror` | `2.0` | Ergonomic `MaterialError` derivation |
| `glam` | `0.32` | Vec2/Vec4 conversions for color utilities |

Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_material_has_reasonable_values() {
        let mat = MaterialDef::default();
        assert_eq!(mat.name, "default");
        // Albedo should be a visible, non-black color
        assert!(mat.albedo[0] > 0.0 && mat.albedo[0] <= 1.0);
        assert!(mat.albedo[1] > 0.0 && mat.albedo[1] <= 1.0);
        assert!(mat.albedo[2] > 0.0 && mat.albedo[2] <= 1.0);
        assert_eq!(mat.albedo[3], 1.0); // fully opaque alpha
        // Medium roughness, non-metallic
        assert_eq!(mat.metallic, 0.0);
        assert!(mat.roughness > 0.0 && mat.roughness < 1.0);
        // No emission
        assert_eq!(mat.emissive_intensity, 0.0);
        // Full opacity
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
        let too_high = MaterialDef { metallic: 1.5, ..Default::default() }
            .validated().unwrap();
        assert_eq!(too_high.metallic, 1.0);

        let too_low = MaterialDef { metallic: -0.2, ..Default::default() }
            .validated().unwrap();
        assert_eq!(too_low.metallic, 0.0);

        let valid = MaterialDef { metallic: 0.7, ..Default::default() }
            .validated().unwrap();
        assert_eq!(valid.metallic, 0.7);
    }

    #[test]
    fn test_roughness_clamped_to_unit_range() {
        let too_high = MaterialDef { roughness: 2.0, ..Default::default() }
            .validated().unwrap();
        assert_eq!(too_high.roughness, 1.0);

        let too_low = MaterialDef { roughness: -0.5, ..Default::default() }
            .validated().unwrap();
        assert_eq!(too_low.roughness, 0.0);
    }

    #[test]
    fn test_emissive_intensity_non_negative() {
        let negative = MaterialDef { emissive_intensity: -5.0, ..Default::default() }
            .validated().unwrap();
        assert_eq!(negative.emissive_intensity, 0.0);

        // Positive values (including > 1.0 for HDR) are allowed
        let hdr = MaterialDef { emissive_intensity: 10.0, ..Default::default() }
            .validated().unwrap();
        assert_eq!(hdr.emissive_intensity, 10.0);
    }

    #[test]
    fn test_material_id_is_unique() {
        // Verify that two distinct MaterialId values are not equal
        let id_a = MaterialId(0);
        let id_b = MaterialId(1);
        assert_ne!(id_a, id_b);

        // Verify that the same value produces equality
        let id_c = MaterialId(42);
        let id_d = MaterialId(42);
        assert_eq!(id_c, id_d);
    }

    #[test]
    fn test_material_gpu_data_size_and_alignment() {
        // MaterialGpuData must be exactly 48 bytes for the storage buffer layout
        assert_eq!(std::mem::size_of::<MaterialGpuData>(), 48);
        // Alignment must be at least 4 (f32 alignment); 16 is preferred for GPU
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
}
```
