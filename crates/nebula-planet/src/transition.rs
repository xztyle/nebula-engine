//! Surface-to-orbit transition: altitude-based rendering mode selection,
//! visual blending, and chunk budget management.
//!
//! This module bridges the gap between voxel terrain rendering (surface) and
//! the orbital textured sphere, providing smooth visual transitions as the
//! camera ascends or descends.

use bytemuck::{Pod, Zeroable};
use nebula_lod::PlanetRenderMode;

/// Configuration for surface-to-orbit transition altitude zones.
#[derive(Clone, Debug)]
pub struct TransitionConfig {
    /// Altitude (meters) below which only voxel rendering is used.
    pub surface_ceiling: f64,
    /// Altitude (meters) above which only orbital rendering is used.
    pub orbital_floor: f64,
}

impl Default for TransitionConfig {
    fn default() -> Self {
        Self {
            surface_ceiling: 50_000.0, // 50 km
            orbital_floor: 200_000.0,  // 200 km
        }
    }
}

/// Smoothstep interpolation: 3t² − 2t³ for t in [0, 1].
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl TransitionConfig {
    /// Determine the render mode and blend factor for a given altitude.
    ///
    /// Returns the appropriate [`PlanetRenderMode`] and a blend factor in \[0, 1\]
    /// where 0 = fully surface (voxels) and 1 = fully orbital (sphere).
    pub fn classify(&self, altitude: f64) -> (PlanetRenderMode, f32) {
        if altitude <= self.surface_ceiling {
            (PlanetRenderMode::VoxelTerrain, 0.0)
        } else if altitude >= self.orbital_floor {
            (PlanetRenderMode::GeometricSphere, 1.0)
        } else {
            let t = ((altitude - self.surface_ceiling)
                / (self.orbital_floor - self.surface_ceiling)) as f32;
            let blend = smoothstep(t);
            (PlanetRenderMode::HybridTerrainSphere, blend)
        }
    }
}

/// GPU uniform for controlling the surface-to-orbit visual blend.
///
/// Both the voxel and orbital render passes read this uniform to modulate
/// their fragment alpha. During transition, voxels fade out while the sphere
/// fades in.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct TransitionUniform {
    /// 0.0 = fully surface (voxels opaque, sphere hidden).
    /// 1.0 = fully orbital (voxels hidden, sphere opaque).
    pub blend_factor: f32,
    /// Padding to 16-byte alignment.
    pub _padding: [f32; 3],
}

impl TransitionUniform {
    /// Create a new transition uniform from a blend factor.
    pub fn new(blend_factor: f32) -> Self {
        Self {
            blend_factor,
            _padding: [0.0; 3],
        }
    }
}

/// Compute the maximum number of active chunks based on altitude.
///
/// Near the surface, the budget is high (many detailed chunks).
/// At orbital altitude, the budget drops to zero (no voxel chunks needed).
/// Uses a quadratic falloff for a smooth budget decrease.
pub fn chunk_budget_for_altitude(altitude: f64, config: &TransitionConfig) -> u32 {
    if altitude >= config.orbital_floor {
        return 0;
    }
    if altitude <= config.surface_ceiling {
        return 4096; // Maximum surface budget
    }

    let t = (altitude - config.surface_ceiling) / (config.orbital_floor - config.surface_ceiling);
    let budget = 4096.0 * (1.0 - t * t); // Quadratic falloff
    budget.max(0.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_visual_discontinuity_during_ascent() {
        let config = TransitionConfig::default();

        let mut prev_blend = 0.0_f32;
        let steps = 1000;
        for i in 0..=steps {
            let alt = i as f64 / steps as f64 * 300_000.0;
            let (_mode, blend) = config.classify(alt);

            // Blend should only increase during ascent (monotonic).
            assert!(
                blend >= prev_blend - 1e-6,
                "Blend decreased at altitude {alt}m: {prev_blend} -> {blend}"
            );

            // No large jumps.
            let delta = (blend - prev_blend).abs();
            assert!(delta < 0.05, "Blend jumped by {delta} at altitude {alt}m");

            prev_blend = blend;
        }
    }

    #[test]
    fn test_chunk_count_decreases_with_altitude() {
        let config = TransitionConfig::default();

        let budget_surface = chunk_budget_for_altitude(1_000.0, &config);
        let budget_mid = chunk_budget_for_altitude(100_000.0, &config);
        let budget_orbit = chunk_budget_for_altitude(300_000.0, &config);

        assert!(
            budget_surface > budget_mid,
            "Surface budget ({budget_surface}) should exceed mid-altitude ({budget_mid})"
        );
        assert!(
            budget_mid > budget_orbit,
            "Mid-altitude budget ({budget_mid}) should exceed orbital ({budget_orbit})"
        );
        assert_eq!(
            budget_orbit, 0,
            "Orbital budget should be 0, got {budget_orbit}"
        );
    }

    #[test]
    fn test_frame_rate_stable_during_transition() {
        let config = TransitionConfig::default();
        let altitude_step = 100.0;

        let mut prev_budget = chunk_budget_for_altitude(0.0, &config);
        let mut max_delta = 0_u32;

        let mut alt = 0.0;
        while alt <= config.orbital_floor + 1000.0 {
            let budget = chunk_budget_for_altitude(alt, &config);
            let delta = prev_budget.abs_diff(budget);
            max_delta = max_delta.max(delta);
            prev_budget = budget;
            alt += altitude_step;
        }

        assert!(
            max_delta < 50,
            "Chunk budget changed by {max_delta} in a single 100m step — too abrupt"
        );
    }

    #[test]
    fn test_transition_is_reversible() {
        let config = TransitionConfig::default();

        let (mode_at_100km, _) = config.classify(100_000.0);
        let (mode_at_10km, _) = config.classify(10_000.0);

        assert_eq!(
            mode_at_100km,
            PlanetRenderMode::HybridTerrainSphere,
            "100 km should be in transition zone"
        );
        assert_eq!(
            mode_at_10km,
            PlanetRenderMode::VoxelTerrain,
            "10 km should be back in surface mode"
        );

        // Blend factor at the same altitude should be identical regardless of direction.
        let ascending = config.classify(125_000.0);
        let descending = config.classify(125_000.0);
        assert_eq!(ascending, descending, "Transition should be symmetric");
    }

    #[test]
    fn test_transition_uniform_alignment() {
        assert_eq!(std::mem::size_of::<TransitionUniform>() % 16, 0);
    }

    #[test]
    fn test_smoothstep_boundary_values() {
        assert!((smoothstep(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((smoothstep(1.0) - 1.0).abs() < f32::EPSILON);
        assert!((smoothstep(0.5) - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_classify_at_boundaries() {
        let config = TransitionConfig::default();

        let (mode, blend) = config.classify(0.0);
        assert_eq!(mode, PlanetRenderMode::VoxelTerrain);
        assert_eq!(blend, 0.0);

        let (mode, blend) = config.classify(50_000.0);
        assert_eq!(mode, PlanetRenderMode::VoxelTerrain);
        assert_eq!(blend, 0.0);

        let (mode, blend) = config.classify(200_000.0);
        assert_eq!(mode, PlanetRenderMode::GeometricSphere);
        assert_eq!(blend, 1.0);

        let (mode, _) = config.classify(125_000.0);
        assert_eq!(mode, PlanetRenderMode::HybridTerrainSphere);
    }
}
