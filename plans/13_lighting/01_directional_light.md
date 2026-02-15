# Directional Light

## Problem

A voxel planet needs a primary light source — the sun — to establish the fundamental day/night cycle and give surfaces their basic shading. Without a directional light, every surface would be rendered at uniform brightness, making terrain look flat and unreadable. The directional light must behave like a real sun: infinitely far away so all rays are parallel, with a direction that rotates relative to the planet's surface to simulate planetary rotation. The light's properties (direction, color, intensity) need to reach the GPU as a uniform buffer that the PBR fragment shader can sample every frame. Getting this foundation right is critical because every subsequent lighting story — shadows, PBR shading, atmospheric scattering — depends on a correct directional light.

## Solution

### Data Structures

Define a `DirectionalLight` component and a corresponding GPU-side uniform in the `nebula_lighting` crate:

```rust
/// CPU-side directional light description.
#[derive(Clone, Debug)]
pub struct DirectionalLight {
    /// Normalized direction vector pointing FROM the light (toward the surface).
    /// In planet-local space so it rotates with the planet.
    pub direction: glam::Vec3,
    /// Linear RGB color of the light (not premultiplied by intensity).
    pub color: glam::Vec3,
    /// Scalar intensity multiplier. Physical range is [0.0, ...), typically 1.0-10.0.
    pub intensity: f32,
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self {
            // Sun slightly off-vertical for interesting initial shading.
            direction: glam::Vec3::new(0.0, -1.0, 0.0).normalize(),
            // Warm white, approximating D65 daylight.
            color: glam::Vec3::new(1.0, 0.96, 0.90),
            intensity: 1.0,
        }
    }
}
```

### Direction Normalization

The direction vector must always be unit length. A `set_direction` method normalizes the input and rejects zero-length vectors:

```rust
impl DirectionalLight {
    pub fn set_direction(&mut self, dir: glam::Vec3) {
        let len = dir.length();
        assert!(len > 1e-6, "directional light direction must not be zero");
        self.direction = dir / len;
    }
}
```

### Planet-Relative Rotation

The sun's direction is defined in the planet's local coordinate frame. As the planet rotates (or equivalently, as time advances), the engine applies a rotation quaternion to the base sun direction:

```rust
pub fn sun_direction_at_time(
    base_direction: glam::Vec3,
    planet_rotation: glam::Quat,
) -> glam::Vec3 {
    (planet_rotation * base_direction).normalize()
}
```

This is evaluated once per frame and written into the uniform buffer.

### GPU Uniform Buffer

```rust
/// GPU-side representation, 32 bytes, std140-compatible.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DirectionalLightUniform {
    /// xyz = direction (normalized), w = intensity.
    pub direction_intensity: [f32; 4],
    /// xyz = color (linear RGB), w = padding.
    pub color_padding: [f32; 4],
}
```

The uniform is written to a `wgpu::Buffer` with `BufferUsages::UNIFORM | BufferUsages::COPY_DST` and updated via `queue.write_buffer()` each frame. The bind group layout places it at `@group(1) @binding(0)`, visible to `ShaderStages::FRAGMENT`.

### WGSL Shader Access

```wgsl
struct DirectionalLight {
    direction_intensity: vec4<f32>,  // xyz = direction, w = intensity
    color_padding: vec4<f32>,        // xyz = color, w = unused
};

@group(1) @binding(0)
var<uniform> sun: DirectionalLight;

fn directional_contribution(normal: vec3<f32>) -> vec3<f32> {
    let n_dot_l = max(dot(normal, -sun.direction_intensity.xyz), 0.0);
    return sun.color_padding.xyz * sun.direction_intensity.w * n_dot_l;
}
```

### Single-Light Constraint

The engine supports exactly one primary directional light (the sun). This simplifies the uniform layout and shadow map pipeline. Additional directional lights (e.g., a moon) can be added later by extending the uniform buffer, but this story scopes to a single source.

## Outcome

A `DirectionalLight` struct and `DirectionalLightUniform` GPU representation in `nebula_lighting`. The light direction is planet-relative and updates each frame to simulate day/night. A uniform buffer is created, bound, and written per frame. The PBR fragment shader reads this buffer to compute basic N-dot-L diffuse contribution. Running `cargo test -p nebula_lighting` passes all directional light tests. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

The sun becomes a proper directional light. Terrain facing the sun is bright; terrain facing away is dark. Basic dot-product lighting gives the terrain 3D form.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | Uniform buffer creation and bind group layout |
| `bytemuck` | `1.21` | Pod/Zeroable derives for GPU struct |
| `glam` | `0.29` | Vec3, Quat for direction and rotation math |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_direction_is_normalized() {
        let light = DirectionalLight::default();
        let len = light.direction.length();
        assert!((len - 1.0).abs() < 1e-6, "direction must be unit length, got {len}");
    }

    #[test]
    fn test_set_direction_normalizes() {
        let mut light = DirectionalLight::default();
        light.set_direction(glam::Vec3::new(3.0, -4.0, 0.0));
        let len = light.direction.length();
        assert!((len - 1.0).abs() < 1e-6, "set_direction must normalize, got {len}");
    }

    #[test]
    #[should_panic(expected = "must not be zero")]
    fn test_zero_direction_panics() {
        let mut light = DirectionalLight::default();
        light.set_direction(glam::Vec3::ZERO);
    }

    #[test]
    fn test_uniform_buffer_layout_matches_shader() {
        // The GPU struct must be exactly 32 bytes (two vec4<f32>).
        assert_eq!(std::mem::size_of::<DirectionalLightUniform>(), 32);
        // Verify field offsets for std140 alignment.
        assert_eq!(
            std::mem::offset_of!(DirectionalLightUniform, direction_intensity),
            0
        );
        assert_eq!(
            std::mem::offset_of!(DirectionalLightUniform, color_padding),
            16
        );
    }

    #[test]
    fn test_default_sun_color_is_warm_white() {
        let light = DirectionalLight::default();
        // Warm white: R >= G >= B, all close to 1.0.
        assert!(light.color.x >= light.color.y, "R should be >= G");
        assert!(light.color.y >= light.color.z, "G should be >= B");
        assert!(light.color.x > 0.9, "R should be near 1.0");
        assert!(light.color.z > 0.8, "B should not be too dim");
    }

    #[test]
    fn test_intensity_in_valid_range() {
        let light = DirectionalLight::default();
        assert!(light.intensity > 0.0, "intensity must be positive");
        assert!(light.intensity.is_finite(), "intensity must be finite");
    }

    #[test]
    fn test_direction_updates_with_planet_rotation() {
        let base = glam::Vec3::new(0.0, -1.0, 0.0);
        // Rotate 90 degrees around Z axis.
        let rotation = glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let rotated = sun_direction_at_time(base, rotation);
        // After 90-degree Z rotation, (0,-1,0) becomes (1,0,0).
        assert!((rotated.x - 1.0).abs() < 1e-5);
        assert!(rotated.y.abs() < 1e-5);
        assert!(rotated.z.abs() < 1e-5);
        // Must still be normalized.
        assert!((rotated.length() - 1.0).abs() < 1e-6);
    }
}
```
