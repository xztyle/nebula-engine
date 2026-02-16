//! CPU-side atmosphere scattering math: parameters, uniform buffer, and ray marching.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::f32::consts::PI;

/// Physical parameters defining a planet's atmosphere.
#[derive(Clone, Debug)]
pub struct AtmosphereParams {
    /// Inner radius: the planet's surface in meters.
    pub planet_radius: f32,
    /// Outer radius: top of the atmosphere in meters.
    pub atmosphere_radius: f32,
    /// Rayleigh scattering coefficients at sea level (per-wavelength, RGB).
    pub rayleigh_coefficients: [f32; 3],
    /// Rayleigh scale height in meters.
    pub rayleigh_scale_height: f32,
    /// Mie scattering coefficient at sea level (scalar).
    pub mie_coefficient: f32,
    /// Mie scale height in meters.
    pub mie_scale_height: f32,
    /// Mie preferred scattering direction (Henyey-Greenstein g parameter).
    pub mie_direction: f32,
    /// Sun intensity multiplier.
    pub sun_intensity: f32,
}

impl AtmosphereParams {
    /// Earth-like atmosphere parameters for a planet of the given radius.
    pub fn earth_like(planet_radius: f32) -> Self {
        Self {
            planet_radius,
            atmosphere_radius: planet_radius * 1.025,
            rayleigh_coefficients: [5.5e-6, 13.0e-6, 22.4e-6],
            rayleigh_scale_height: 8500.0,
            mie_coefficient: 21e-6,
            mie_scale_height: 1200.0,
            mie_direction: 0.758,
            sun_intensity: 22.0,
        }
    }
}

/// GPU-side atmosphere uniform buffer. Matches the WGSL struct layout.
///
/// WGSL alignment rules: vec3<f32> has 16-byte alignment, so we must
/// insert explicit padding after scalar fields that precede a vec3.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct AtmosphereUniform {
    /// Planet center in world space. (offset 0)
    pub planet_center: [f32; 3],
    /// Planet surface radius. (offset 12)
    pub planet_radius: f32,
    /// Atmosphere outer radius. (offset 16)
    pub atmosphere_radius: f32,
    /// Padding to align rayleigh_coefficients to 16 bytes. (offset 20)
    pub _pad_rc: [f32; 3],
    /// Rayleigh scattering coefficients (RGB). (offset 32)
    pub rayleigh_coefficients: [f32; 3],
    /// Rayleigh scale height. (offset 44)
    pub rayleigh_scale_height: f32,
    /// Mie scattering coefficient. (offset 48)
    pub mie_coefficient: f32,
    /// Mie scale height. (offset 52)
    pub mie_scale_height: f32,
    /// Mie direction (g parameter). (offset 56)
    pub mie_direction: f32,
    /// Padding to align sun_direction to 16 bytes. (offset 60)
    pub _pad_sd: f32,
    /// Normalized sun direction. (offset 64)
    pub sun_direction: [f32; 3],
    /// Sun intensity. (offset 76)
    pub sun_intensity: f32,
    /// Camera position in world space. (offset 80)
    pub camera_position: [f32; 3],
    /// Padding for 16-byte alignment before mat4. (offset 92)
    pub _padding0: f32,
    /// Inverse view-projection matrix (column-major). (offset 96)
    pub inv_view_proj: [[f32; 4]; 4],
    /// Near clip plane distance. (offset 160)
    pub near_clip: f32,
    /// Far clip plane distance. (offset 164)
    pub far_clip: f32,
    /// Padding. (offset 168)
    pub _padding1: [f32; 2],
}

impl AtmosphereUniform {
    /// Create a uniform from parameters and per-frame state.
    pub fn from_params(
        params: &AtmosphereParams,
        planet_center: Vec3,
        sun_direction: Vec3,
        camera_position: Vec3,
        inv_view_proj: glam::Mat4,
        near_clip: f32,
        far_clip: f32,
    ) -> Self {
        Self {
            planet_center: planet_center.to_array(),
            planet_radius: params.planet_radius,
            atmosphere_radius: params.atmosphere_radius,
            _pad_rc: [0.0; 3],
            rayleigh_coefficients: params.rayleigh_coefficients,
            rayleigh_scale_height: params.rayleigh_scale_height,
            mie_coefficient: params.mie_coefficient,
            mie_scale_height: params.mie_scale_height,
            mie_direction: params.mie_direction,
            _pad_sd: 0.0,
            sun_direction: sun_direction.normalize().to_array(),
            sun_intensity: params.sun_intensity,
            camera_position: camera_position.to_array(),
            _padding0: 0.0,
            inv_view_proj: inv_view_proj.to_cols_array_2d(),
            near_clip,
            far_clip,
            _padding1: [0.0; 2],
        }
    }
}

/// Ray-sphere intersection returning (t_near, t_far). Returns (-1, -1) on miss.
pub fn ray_sphere_intersect_f32(origin: Vec3, dir: Vec3, center: Vec3, radius: f32) -> (f32, f32) {
    let oc = origin - center;
    let b = oc.dot(dir);
    let c = oc.dot(oc) - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return (-1.0, -1.0);
    }
    let sqrt_disc = disc.sqrt();
    (-b - sqrt_disc, -b + sqrt_disc)
}

/// Compute single-scattering atmosphere color for a given view ray.
///
/// Used for CPU-side validation and unit tests. The GPU shader implements
/// the same algorithm.
pub fn compute_single_scatter(
    camera_pos: Vec3,
    look_dir: Vec3,
    sun_dir: Vec3,
    params: &AtmosphereParams,
    num_samples: u32,
    num_light_samples: u32,
) -> [f32; 3] {
    let planet_center = Vec3::ZERO;

    let (t_near, t_far) = ray_sphere_intersect_f32(
        camera_pos,
        look_dir,
        planet_center,
        params.atmosphere_radius,
    );

    if t_far < 0.0 || t_near > t_far {
        return [0.0; 3];
    }

    // Check if ray hits the planet surface
    let (planet_near, _) =
        ray_sphere_intersect_f32(camera_pos, look_dir, planet_center, params.planet_radius);

    let t_start = t_near.max(0.0);
    let t_end = if planet_near > 0.0 {
        t_far.min(planet_near)
    } else {
        t_far
    };

    if t_end <= t_start {
        return [0.0; 3];
    }

    let step_size = (t_end - t_start) / num_samples as f32;
    let cos_angle = look_dir.dot(sun_dir);
    let phase_r = rayleigh_phase(cos_angle);
    let phase_m = mie_phase(cos_angle, params.mie_direction);

    let mut total_rayleigh = Vec3::ZERO;
    let mut total_mie = Vec3::ZERO;
    let mut optical_depth_r = 0.0_f32;
    let mut optical_depth_m = 0.0_f32;

    let rc = Vec3::from(params.rayleigh_coefficients);

    for i in 0..num_samples {
        let t = t_start + (i as f32 + 0.5) * step_size;
        let sample_pos = camera_pos + look_dir * t;
        let height = (sample_pos - planet_center).length() - params.planet_radius;

        let density_r = (-height / params.rayleigh_scale_height).exp() * step_size;
        let density_m = (-height / params.mie_scale_height).exp() * step_size;

        optical_depth_r += density_r;
        optical_depth_m += density_m;

        // Light ray march toward sun
        let (_, light_far) =
            ray_sphere_intersect_f32(sample_pos, sun_dir, planet_center, params.atmosphere_radius);
        let light_step = light_far / num_light_samples as f32;
        let mut light_depth_r = 0.0_f32;
        let mut light_depth_m = 0.0_f32;

        for j in 0..num_light_samples {
            let lt = (j as f32 + 0.5) * light_step;
            let light_pos = sample_pos + sun_dir * lt;
            let light_height = (light_pos - planet_center).length() - params.planet_radius;
            light_depth_r += (-light_height / params.rayleigh_scale_height).exp() * light_step;
            light_depth_m += (-light_height / params.mie_scale_height).exp() * light_step;
        }

        let tau = rc * (optical_depth_r + light_depth_r)
            + Vec3::splat(params.mie_coefficient) * (optical_depth_m + light_depth_m);
        let attenuation = Vec3::new((-tau.x).exp(), (-tau.y).exp(), (-tau.z).exp());

        total_rayleigh += Vec3::splat(density_r) * attenuation;
        total_mie += Vec3::splat(density_m) * attenuation;
    }

    let color = params.sun_intensity
        * (phase_r * rc * total_rayleigh + phase_m * params.mie_coefficient * total_mie);

    color.to_array()
}

fn rayleigh_phase(cos_angle: f32) -> f32 {
    3.0 / (16.0 * PI) * (1.0 + cos_angle * cos_angle)
}

fn mie_phase(cos_angle: f32, g: f32) -> f32 {
    let g2 = g * g;
    let num = 3.0 * (1.0 - g2) * (1.0 + cos_angle * cos_angle);
    let denom = 8.0 * PI * (2.0 + g2) * (1.0 + g2 - 2.0 * g * cos_angle).powf(1.5);
    num / denom
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atmosphere_visible_from_surface() {
        let params = AtmosphereParams::earth_like(6_371_000.0);
        let camera_pos = Vec3::new(0.0, params.planet_radius + 1.7, 0.0);
        let look_up = Vec3::Y;

        let (t_near, t_far) =
            ray_sphere_intersect_f32(camera_pos, look_up, Vec3::ZERO, params.atmosphere_radius);

        assert!(t_far > t_near, "Atmosphere should be intersected");
        assert!(t_far > 0.0, "Intersection should be in front of camera");

        let path_length = t_far - t_near.max(0.0);
        let expected = params.atmosphere_radius - params.planet_radius;
        assert!(
            (path_length - expected).abs() / expected < 0.01,
            "Path {path_length} should be ~{expected}"
        );
    }

    #[test]
    fn test_atmosphere_visible_from_orbit() {
        // Use a small planet to avoid f32 precision issues
        let params = AtmosphereParams::earth_like(1000.0);
        let camera_pos = Vec3::new(0.0, params.atmosphere_radius * 3.0, 0.0);
        let look_tangent = Vec3::new(0.3, -1.0, 0.0).normalize();

        let (t_near, t_far) = ray_sphere_intersect_f32(
            camera_pos,
            look_tangent,
            Vec3::ZERO,
            params.atmosphere_radius,
        );

        assert!(
            t_far > t_near && t_near > 0.0,
            "Atmosphere visible from orbit: t_near={t_near}, t_far={t_far}"
        );
    }

    #[test]
    fn test_sky_color_changes_with_sun_angle() {
        let params = AtmosphereParams::earth_like(6_371_000.0);
        let camera_pos = Vec3::new(0.0, params.planet_radius + 1.7, 0.0);
        let look_dir = Vec3::new(1.0, 0.5, 0.0).normalize();

        let color_noon = compute_single_scatter(camera_pos, look_dir, Vec3::Y, &params, 16, 8);
        let color_sunset = compute_single_scatter(
            camera_pos,
            look_dir,
            Vec3::new(1.0, 0.01, 0.0).normalize(),
            &params,
            16,
            8,
        );

        assert!(color_noon[2] > color_noon[0], "Noon sky should be bluer");

        let noon_ratio = color_noon[0] / color_noon[2].max(1e-10);
        let sunset_ratio = color_sunset[0] / color_sunset[2].max(1e-10);
        assert!(
            sunset_ratio > noon_ratio,
            "Sunset red/blue ratio {sunset_ratio:.3} > noon {noon_ratio:.3}"
        );
    }

    #[test]
    fn test_atmosphere_differs_up_vs_horizon() {
        let params = AtmosphereParams::earth_like(6_371_000.0);
        let camera_pos = Vec3::new(0.0, params.planet_radius + 1.7, 0.0);
        let sun_dir = Vec3::Y;

        let color_up = compute_single_scatter(camera_pos, Vec3::Y, sun_dir, &params, 16, 8);
        let color_horiz = compute_single_scatter(camera_pos, Vec3::X, sun_dir, &params, 16, 8);

        let b_up: f32 = color_up.iter().sum();
        let b_hz: f32 = color_horiz.iter().sum();

        assert!(
            (b_up - b_hz).abs() > b_up * 0.1,
            "up={b_up}, horizon={b_hz} should differ"
        );
    }

    #[test]
    fn test_no_boundary_discontinuity() {
        let params = AtmosphereParams::earth_like(6_371_000.0);
        let camera_pos = Vec3::new(0.0, params.planet_radius + 1.7, 0.0);
        let sun_dir = Vec3::Y;

        let angles = [0.01_f32, 0.02, 0.03, 0.04, 0.05];
        let colors: Vec<[f32; 3]> = angles
            .iter()
            .map(|&a| {
                let dir = Vec3::new(a.cos(), a.sin(), 0.0);
                compute_single_scatter(camera_pos, dir, sun_dir, &params, 16, 8)
            })
            .collect();

        for i in 1..colors.len() {
            let diff: f32 = (0..3)
                .map(|c| (colors[i][c] - colors[i - 1][c]).abs())
                .sum();
            let avg: f32 = (0..3).map(|c| colors[i][c]).sum::<f32>().max(1e-6);
            assert!(diff / avg < 0.5, "Discontinuity at index {i}");
        }
    }

    #[test]
    fn test_uniform_alignment() {
        assert_eq!(std::mem::size_of::<AtmosphereUniform>() % 16, 0);
    }

    #[test]
    fn test_ray_sphere_miss() {
        let (t_near, t_far) =
            ray_sphere_intersect_f32(Vec3::new(0.0, 10.0, 0.0), Vec3::X, Vec3::ZERO, 1.0);
        assert!(t_near < 0.0 || t_near > t_far);
    }
}
