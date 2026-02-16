//! Ocean surface rendering: animated waves, Fresnel reflections, and depth-dependent coloring.

mod renderer;

pub use renderer::OceanRenderer;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

/// Ocean configuration for a planet.
#[derive(Clone, Debug)]
pub struct OceanParams {
    /// Sea level offset from planet surface radius, in meters.
    pub sea_level: f64,
    /// Deep ocean color (linear RGB). Default: dark blue.
    pub deep_color: [f32; 3],
    /// Shallow water color (linear RGB). Default: turquoise.
    pub shallow_color: [f32; 3],
    /// Depth at which water transitions from shallow to deep color, in meters.
    pub color_depth: f32,
    /// Wave amplitude in meters.
    pub wave_amplitude: f32,
    /// Wave frequency (cycles per meter).
    pub wave_frequency: f32,
    /// Wave speed (meters per second).
    pub wave_speed: f32,
    /// Fresnel reflectance at normal incidence (F0). Water is ~0.02.
    pub fresnel_f0: f32,
}

impl Default for OceanParams {
    fn default() -> Self {
        Self {
            sea_level: 0.0,
            deep_color: [0.01, 0.03, 0.15],
            shallow_color: [0.0, 0.5, 0.6],
            color_depth: 50.0,
            wave_amplitude: 0.5,
            wave_frequency: 0.05,
            wave_speed: 2.0,
            fresnel_f0: 0.02,
        }
    }
}

/// GPU uniform for ocean rendering.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct OceanUniform {
    /// Deep ocean color (linear RGB).
    pub deep_color: [f32; 3],
    /// Depth at which water transitions from shallow to deep color.
    pub color_depth: f32,
    /// Shallow water color (linear RGB).
    pub shallow_color: [f32; 3],
    /// Wave amplitude in meters.
    pub wave_amplitude: f32,
    /// Wave frequency (cycles per meter).
    pub wave_frequency: f32,
    /// Wave speed (meters per second).
    pub wave_speed: f32,
    /// Fresnel reflectance at normal incidence (F0).
    pub fresnel_f0: f32,
    /// Current simulation time in seconds.
    pub time: f32,
    /// Normalized sun direction in world space.
    pub sun_direction: [f32; 3],
    /// Ocean sphere radius (planet_radius + sea_level).
    pub ocean_radius: f32,
    /// Camera position in world space.
    pub camera_position: [f32; 3],
    /// Padding for 16-byte alignment.
    pub _padding: f32,
}

impl OceanUniform {
    /// Create a uniform from ocean params and per-frame state.
    pub fn from_params(
        params: &OceanParams,
        sun_direction: Vec3,
        camera_position: Vec3,
        ocean_radius: f32,
        time: f32,
    ) -> Self {
        Self {
            deep_color: params.deep_color,
            color_depth: params.color_depth,
            shallow_color: params.shallow_color,
            wave_amplitude: params.wave_amplitude,
            wave_frequency: params.wave_frequency,
            wave_speed: params.wave_speed,
            fresnel_f0: params.fresnel_f0,
            time,
            sun_direction: sun_direction.normalize().to_array(),
            ocean_radius,
            camera_position: camera_position.to_array(),
            _padding: 0.0,
        }
    }
}

/// Compute wave displacement at a world position for a given time.
///
/// Two overlapping sine waves create a more natural ripple pattern.
pub fn compute_wave_displacement(position: &Vec3, time: f32, params: &OceanParams) -> f32 {
    let wave1 = (position.x * params.wave_frequency + time * params.wave_speed).sin()
        * params.wave_amplitude;
    let wave2 = ((position.x * 0.7 + position.z * 0.7) * params.wave_frequency * 1.3
        + time * params.wave_speed * 0.8)
        .sin()
        * params.wave_amplitude
        * 0.5;
    wave1 + wave2
}

/// Compute water color based on depth, blending shallow to deep.
pub fn compute_water_color(depth: f32, params: &OceanParams) -> [f32; 3] {
    let t = (depth / params.color_depth).clamp(0.0, 1.0);
    [
        params.shallow_color[0] + (params.deep_color[0] - params.shallow_color[0]) * t,
        params.shallow_color[1] + (params.deep_color[1] - params.shallow_color[1]) * t,
        params.shallow_color[2] + (params.deep_color[2] - params.shallow_color[2]) * t,
    ]
}

/// Ray-sphere intersection returning (t_near, t_far). Negative values mean no hit.
pub fn ray_sphere_intersect_f32(
    origin: Vec3,
    direction: Vec3,
    center: Vec3,
    radius: f32,
) -> (f32, f32) {
    let oc = origin - center;
    let b = oc.dot(direction);
    let c = oc.dot(oc) - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return (-1.0, -1.0);
    }
    let sqrt_disc = disc.sqrt();
    (-b - sqrt_disc, -b + sqrt_disc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ocean_visible_at_sea_level() {
        let params = OceanParams::default();
        let planet_radius = 6_371_000.0_f32;
        let ocean_radius = planet_radius + params.sea_level as f32;

        let camera_pos = Vec3::new(0.0, planet_radius + 1.7, 0.0);
        let look_dir = Vec3::new(1.0, -0.01, 0.0).normalize();

        let (t_near, t_far) =
            ray_sphere_intersect_f32(camera_pos, look_dir, Vec3::ZERO, ocean_radius);
        assert!(
            t_far > 0.0 && t_near < t_far,
            "Ocean sphere should be intersected from surface: t_near={t_near}, t_far={t_far}"
        );
    }

    #[test]
    fn test_ocean_hides_terrain_below_sea_level() {
        let params = OceanParams::default();
        let planet_radius = 1000.0_f32;
        let sea_level = params.sea_level as f32;
        let ocean_radius = planet_radius + sea_level;

        let terrain_height = sea_level - 50.0;
        let terrain_surface = planet_radius + terrain_height;

        assert!(
            ocean_radius > terrain_surface,
            "Ocean ({ocean_radius}) should be above submerged terrain ({terrain_surface})"
        );

        let water_depth = ocean_radius - terrain_surface;
        assert!(
            water_depth > 0.0,
            "Water depth should be positive for submerged terrain, got {water_depth}"
        );
    }

    #[test]
    fn test_waves_animate_over_time() {
        let params = OceanParams::default();
        let position = Vec3::new(100.0, 0.0, 50.0);
        let displacements: Vec<f32> = (0..10)
            .map(|i| {
                let time = i as f32 * 0.5;
                compute_wave_displacement(&position, time, &params)
            })
            .collect();

        let all_same = displacements.windows(2).all(|w| (w[0] - w[1]).abs() < 1e-6);
        assert!(
            !all_same,
            "Wave displacement should change over time, but got: {displacements:?}"
        );

        let max_amp = params.wave_amplitude * 1.5;
        for (i, &d) in displacements.iter().enumerate() {
            assert!(
                d.abs() <= max_amp + 0.01,
                "Displacement at time {} exceeds max amplitude: {} > {}",
                i as f32 * 0.5,
                d.abs(),
                max_amp
            );
        }
    }

    #[test]
    fn test_depth_coloring_changes() {
        let params = OceanParams::default();

        let shallow_color = compute_water_color(1.0, &params);
        let deep_color = compute_water_color(500.0, &params);

        assert!(
            shallow_color[1] > deep_color[1],
            "Shallow water should be more green/turquoise: shallow={shallow_color:?}, deep={deep_color:?}"
        );
        // Deep water should be darker overall (lower total luminance).
        let shallow_lum: f32 = shallow_color.iter().sum();
        let deep_lum: f32 = deep_color.iter().sum();
        assert!(
            deep_lum < shallow_lum,
            "Deep water should be darker overall: shallow_lum={shallow_lum}, deep_lum={deep_lum}"
        );
    }

    #[test]
    fn test_ocean_uniform_alignment() {
        assert_eq!(std::mem::size_of::<OceanUniform>() % 16, 0);
    }

    #[test]
    fn test_no_z_fighting_at_shoreline() {
        let bias = OceanRenderer::depth_bias_state();
        assert!(
            bias.constant != 0 || bias.slope_scale != 0.0,
            "Depth bias must be non-zero to prevent z-fighting at shoreline"
        );

        assert!(
            bias.constant < 0,
            "Depth bias constant should be negative for reverse-Z, got {}",
            bias.constant
        );
    }
}
