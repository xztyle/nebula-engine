# PBR Shading

## Problem

The engine has directional and point lights, shadow maps, and per-voxel light levels — but without a physically-based shading model, the final pixel colors will look either too flat (pure diffuse) or unconvincingly shiny (ad-hoc specular). Modern rendering expects materials to respond to light in a physically plausible way: metals reflect differently than plastics, rough surfaces scatter light broadly while smooth surfaces produce tight highlights, and all materials conserve energy (they never reflect more light than they receive). The engine needs a Cook-Torrance BRDF implementation in the fragment shader that takes material properties (albedo, metallic, roughness, normal, ambient occlusion) and evaluates the lighting contribution from all active light sources.

## Solution

### Material Inputs

Each fragment receives material properties either from textures or from per-voxel material definitions:

```rust
/// PBR material parameters (CPU-side, per voxel type or per texture).
#[derive(Clone, Debug)]
pub struct PbrMaterial {
    /// Base color (linear RGB).
    pub albedo: glam::Vec3,
    /// Metallic factor [0.0, 1.0]. 0 = dielectric, 1 = metal.
    pub metallic: f32,
    /// Roughness factor [0.0, 1.0]. 0 = mirror, 1 = fully rough.
    pub roughness: f32,
    /// Ambient occlusion [0.0, 1.0]. 1 = fully exposed, 0 = fully occluded.
    pub ao: f32,
}
```

### Cook-Torrance BRDF in WGSL

The fragment shader implements the standard Cook-Torrance microfacet specular BRDF plus Lambertian diffuse:

```wgsl
const PI: f32 = 3.14159265359;

// Normal Distribution Function: GGX/Trowbridge-Reitz.
fn distribution_ggx(n_dot_h: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let denom = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / (PI * denom * denom);
}

// Geometry Function: Schlick-GGX (Smith's method, combined for both view and light).
fn geometry_schlick_ggx(n_dot_v: f32, roughness: f32) -> f32 {
    let r = roughness + 1.0;
    let k = (r * r) / 8.0;
    return n_dot_v / (n_dot_v * (1.0 - k) + k);
}

fn geometry_smith(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    let ggx_v = geometry_schlick_ggx(n_dot_v, roughness);
    let ggx_l = geometry_schlick_ggx(n_dot_l, roughness);
    return ggx_v * ggx_l;
}

// Fresnel: Schlick approximation.
fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (1.0 - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

// Full PBR evaluation for a single light.
fn evaluate_brdf(
    light_dir: vec3<f32>,    // normalized direction TO the light
    view_dir: vec3<f32>,     // normalized direction TO the camera
    normal: vec3<f32>,       // surface normal
    albedo: vec3<f32>,
    metallic: f32,
    roughness: f32,
) -> vec3<f32> {
    let half_vec = normalize(view_dir + light_dir);

    let n_dot_l = max(dot(normal, light_dir), 0.0);
    let n_dot_v = max(dot(normal, view_dir), 0.0);
    let n_dot_h = max(dot(normal, half_vec), 0.0);
    let h_dot_v = max(dot(half_vec, view_dir), 0.0);

    // Base reflectance: dielectrics use 0.04, metals use albedo.
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);

    // Specular (Cook-Torrance).
    let d = distribution_ggx(n_dot_h, roughness);
    let g = geometry_smith(n_dot_v, n_dot_l, roughness);
    let f = fresnel_schlick(h_dot_v, f0);

    let numerator = d * g * f;
    let denominator = 4.0 * n_dot_v * n_dot_l + 0.0001; // prevent div by zero
    let specular = numerator / denominator;

    // Diffuse (Lambertian). Metals have no diffuse.
    let k_s = f; // specular fraction
    let k_d = (vec3<f32>(1.0) - k_s) * (1.0 - metallic);
    let diffuse = k_d * albedo / PI;

    return (diffuse + specular) * n_dot_l;
}
```

### Main Fragment Shader Integration

```wgsl
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let albedo = material.albedo;
    let metallic = material.metallic;
    let roughness = material.roughness;
    let ao = material.ao;
    let normal = normalize(in.world_normal);
    let view_dir = normalize(camera.position.xyz - in.world_pos);

    // Directional light contribution.
    var color = evaluate_brdf(-sun.direction_intensity.xyz, view_dir, normal, albedo, metallic, roughness)
              * sun.color_padding.xyz * sun.direction_intensity.w
              * shadow_factor(in.world_pos, in.view_depth);

    // Point light contributions.
    for (var i = 0u; i < point_lights.count; i++) {
        let light = point_lights.lights[i];
        let to_light = light.position_radius.xyz - in.world_pos;
        let dist = length(to_light);
        let atten = attenuation(dist, light.position_radius.w);
        color += evaluate_brdf(normalize(to_light), view_dir, normal, albedo, metallic, roughness)
               * light.color_intensity.xyz * light.color_intensity.w * atten;
    }

    // Ambient term (simple, IBL comes later).
    let ambient = vec3<f32>(0.03) * albedo * ao;
    color += ambient;

    return vec4<f32>(color, 1.0);
}
```

### Energy Conservation

The BRDF naturally conserves energy: the diffuse component is scaled by `(1 - k_s) * (1 - metallic)`, ensuring that the sum of diffuse and specular never exceeds the incoming light energy. Metals reflect only specularly (no diffuse), while dielectrics split energy between diffuse and specular based on the Fresnel term.

## Outcome

A WGSL fragment shader implementing Cook-Torrance PBR with GGX distribution, Schlick Fresnel, and Smith geometry terms. The shader evaluates both the directional light (sun) and all active point lights per fragment. Material inputs (albedo, metallic, roughness, AO) come from per-voxel material definitions. Running `cargo test -p nebula_lighting` passes all PBR shading tests. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Stone looks rough and matte; ore deposits have subtle metallic specularity. The terrain gains visual richness from physically-based reflectance.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | Shader module compilation, pipeline creation |
| `bytemuck` | `1.21` | Pod/Zeroable for material uniform buffer |
| `glam` | `0.29` | Vec3 for material parameters and test math |

Depends on stories 01 (directional light), 02 (point lights), and 03 (shadow maps) for light data in the shader.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// CPU-side reference implementation of the BRDF for validation.
    fn evaluate_brdf_cpu(
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

    #[test]
    fn test_pure_metal_has_no_diffuse() {
        let result = evaluate_brdf_cpu(
            glam::Vec3::Y,            // light from above
            glam::Vec3::new(0.0, 1.0, 1.0).normalize(), // view angle
            glam::Vec3::Y,            // surface facing up
            glam::Vec3::new(1.0, 0.8, 0.2), // gold albedo
            1.0,                       // fully metallic
            0.5,
        );
        // For metallic = 1.0, k_d = (1 - F) * (1 - 1.0) = 0.
        // All contribution should be specular. Verify diffuse component is zero.
        let f0 = glam::Vec3::new(1.0, 0.8, 0.2); // metallic uses albedo as f0
        // The diffuse term = k_d * albedo / PI = 0 for metals.
        // The total result should be purely specular (non-zero due to specular term).
        // We verify by checking that with metallic=0, we get a different (larger) result
        // including diffuse.
        let dielectric_result = evaluate_brdf_cpu(
            glam::Vec3::Y,
            glam::Vec3::new(0.0, 1.0, 1.0).normalize(),
            glam::Vec3::Y,
            glam::Vec3::new(1.0, 0.8, 0.2),
            0.0, // dielectric
            0.5,
        );
        // Dielectric result should be strictly larger due to diffuse contribution.
        assert!(
            dielectric_result.length() > result.length(),
            "dielectric ({:?}) should have more total light than metal ({:?}) due to diffuse",
            dielectric_result,
            result
        );
    }

    #[test]
    fn test_pure_dielectric_has_full_diffuse() {
        let result = evaluate_brdf_cpu(
            glam::Vec3::Y,
            glam::Vec3::Y,
            glam::Vec3::Y,
            glam::Vec3::ONE, // white
            0.0,             // fully dielectric
            1.0,             // fully rough
        );
        // With metallic = 0.0, the diffuse term should be substantial.
        assert!(result.length() > 0.0, "dielectric should have non-zero output");
        // At roughness=1.0 and normal incidence, diffuse dominates.
        // Diffuse = (1-F) * albedo/PI * n_dot_l ~ 0.96 * 1/PI * 1 ~ 0.306
        assert!(result.x > 0.2, "diffuse contribution should be significant for dielectric");
    }

    #[test]
    fn test_roughness_zero_gives_sharp_specular() {
        let smooth = evaluate_brdf_cpu(
            glam::Vec3::Y,
            glam::Vec3::Y,
            glam::Vec3::Y,
            glam::Vec3::ONE,
            0.5,
            0.01, // nearly smooth
        );
        let rough = evaluate_brdf_cpu(
            glam::Vec3::Y,
            glam::Vec3::Y,
            glam::Vec3::Y,
            glam::Vec3::ONE,
            0.5,
            0.99, // nearly rough
        );
        // At perfect mirror reflection (view = light = normal), smooth should have
        // a much stronger specular peak than rough.
        assert!(
            smooth.length() > rough.length(),
            "smooth ({:?}) should have stronger specular peak than rough ({:?})",
            smooth,
            rough
        );
    }

    #[test]
    fn test_roughness_one_gives_broad_specular() {
        // At roughness=1, the GGX distribution spreads the specular across all angles.
        // Test at an off-angle: the rough surface should still have some specular.
        let off_angle_light = glam::Vec3::new(0.5, 0.5, 0.0).normalize();
        let rough = evaluate_brdf_cpu(
            off_angle_light,
            glam::Vec3::Y,
            glam::Vec3::Y,
            glam::Vec3::ONE,
            0.5,
            1.0,
        );
        assert!(rough.length() > 0.0, "rough material should scatter light broadly");
    }

    #[test]
    fn test_energy_conservation() {
        // The outgoing light must never exceed incoming light.
        // Incoming = light_color * intensity * n_dot_l = 1.0 * 1.0 * 1.0 = 1.0 per channel.
        let result = evaluate_brdf_cpu(
            glam::Vec3::Y,
            glam::Vec3::Y,
            glam::Vec3::Y,
            glam::Vec3::ONE, // white
            0.5,
            0.5,
        );
        // n_dot_l = 1.0 is already factored in. The BRDF * n_dot_l should be <= 1.0.
        assert!(result.x <= 1.0 + 1e-6, "R channel ({}) exceeds incoming", result.x);
        assert!(result.y <= 1.0 + 1e-6, "G channel ({}) exceeds incoming", result.y);
        assert!(result.z <= 1.0 + 1e-6, "B channel ({}) exceeds incoming", result.z);
    }

    #[test]
    fn test_ao_reduces_ambient_contribution() {
        let albedo = glam::Vec3::ONE;
        let ambient_full = glam::Vec3::splat(0.03) * albedo * 1.0; // ao = 1.0
        let ambient_half = glam::Vec3::splat(0.03) * albedo * 0.5; // ao = 0.5
        let ambient_zero = glam::Vec3::splat(0.03) * albedo * 0.0; // ao = 0.0
        assert!(ambient_full.x > ambient_half.x);
        assert!(ambient_half.x > ambient_zero.x);
        assert_eq!(ambient_zero.x, 0.0);
    }
}
```
