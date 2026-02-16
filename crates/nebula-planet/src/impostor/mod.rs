//! Planet impostor system: billboard rendering for extremely distant planets.
//!
//! When a planet is millions of kilometers away and occupies only a few pixels,
//! rendering a full icosphere is wasteful. This module provides a 2-triangle
//! billboard quad with a pre-rendered snapshot texture.

mod pipeline;

pub use pipeline::{IMPOSTOR_SHADER_SOURCE, ImpostorPipeline, ImpostorRenderer, ImpostorVertex};

use glam::Vec3;

/// Configuration for when to use impostors vs. geometry.
#[derive(Clone, Debug)]
pub struct ImpostorConfig {
    /// Distance (in meters) beyond which the orbital sphere is replaced by an impostor.
    pub impostor_distance: f64,
    /// Transition band (in meters) for blending between sphere and impostor.
    pub transition_band: f64,
    /// Angular change (radians) in view direction that triggers a texture update.
    pub angle_threshold: f32,
    /// Impostor texture resolution in pixels (square).
    pub texture_resolution: u32,
}

impl Default for ImpostorConfig {
    fn default() -> Self {
        Self {
            impostor_distance: 1_000_000_000.0, // 1 million km
            transition_band: 100_000_000.0,     // 100k km
            angle_threshold: 0.05,              // ~2.9 degrees
            texture_resolution: 128,
        }
    }
}

/// The rendering representation selected for a planet at a given distance.
#[derive(Debug, Clone, Copy)]
pub enum PlanetRepresentation {
    /// Full geometry (orbital sphere + terrain).
    Geometry,
    /// Blending between geometry and impostor.
    Blending {
        /// 0.0 = fully geometry, 1.0 = fully impostor.
        impostor_alpha: f32,
    },
    /// Pure impostor billboard.
    Impostor,
}

/// Select the rendering representation based on distance from camera to planet.
pub fn select_planet_representation(
    distance: f64,
    config: &ImpostorConfig,
) -> PlanetRepresentation {
    if distance < config.impostor_distance {
        PlanetRepresentation::Geometry
    } else if distance < config.impostor_distance + config.transition_band {
        let t = ((distance - config.impostor_distance) / config.transition_band) as f32;
        PlanetRepresentation::Blending {
            impostor_alpha: t.clamp(0.0, 1.0),
        }
    } else {
        PlanetRepresentation::Impostor
    }
}

/// Compute the world-space size of the impostor quad.
///
/// The quad subtends the same visual angle as the planet sphere.
pub fn impostor_quad_size(planet_radius: f64, distance: f64) -> f32 {
    let angular_radius = (planet_radius / distance).asin();
    (angular_radius.tan() * distance * 2.0) as f32
}

/// Generate billboard vertices for a camera-facing quad.
///
/// Returns 4 vertices forming a quad centered on `planet_center`,
/// oriented to face the camera using the provided right and up vectors.
pub fn billboard_vertices(
    planet_center: Vec3,
    camera_right: Vec3,
    camera_up: Vec3,
    half_size: f32,
) -> [ImpostorVertex; 4] {
    let r = camera_right * half_size;
    let u = camera_up * half_size;
    [
        ImpostorVertex {
            position: (planet_center - r - u).into(),
            uv: [0.0, 1.0],
        },
        ImpostorVertex {
            position: (planet_center + r - u).into(),
            uv: [1.0, 1.0],
        },
        ImpostorVertex {
            position: (planet_center + r + u).into(),
            uv: [1.0, 0.0],
        },
        ImpostorVertex {
            position: (planet_center - r + u).into(),
            uv: [0.0, 0.0],
        },
    ]
}

/// Index data for the impostor quad (2 triangles).
pub const IMPOSTOR_INDICES: [u16; 6] = [0, 1, 2, 0, 2, 3];

/// Metadata for an impostor's captured snapshot state.
///
/// Tracks the view and sun directions at capture time to determine
/// when the texture needs re-rendering.
#[derive(Clone, Debug)]
pub struct ImpostorState {
    /// The view direction (camera-to-planet normalized) at capture time.
    pub captured_view_dir: Vec3,
    /// The sun direction at capture time.
    pub captured_sun_dir: Vec3,
    /// Angular threshold (radians) before re-rendering.
    pub update_threshold: f32,
    /// Whether the texture needs re-rendering.
    pub dirty: bool,
}

impl ImpostorState {
    /// Create a new impostor state that starts dirty (needs initial capture).
    pub fn new(threshold: f32) -> Self {
        Self {
            captured_view_dir: Vec3::Z,
            captured_sun_dir: Vec3::Y,
            update_threshold: threshold,
            dirty: true,
        }
    }

    /// Check if the impostor texture needs re-rendering.
    pub fn needs_update(&self, current_view_dir: Vec3, current_sun_dir: Vec3) -> bool {
        if self.dirty {
            return true;
        }
        let view_angle_change = self.captured_view_dir.angle_between(current_view_dir);
        let sun_angle_change = self.captured_sun_dir.angle_between(current_sun_dir);
        view_angle_change > self.update_threshold || sun_angle_change > self.update_threshold
    }

    /// Mark the snapshot as up-to-date with the given directions.
    pub fn mark_captured(&mut self, view_dir: Vec3, sun_dir: Vec3) {
        self.captured_view_dir = view_dir;
        self.captured_sun_dir = sun_dir;
        self.dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn test_impostor_replaces_geometry_at_extreme_distance() {
        let config = ImpostorConfig::default();

        let result = select_planet_representation(10_000_000_000_000.0, &config);
        assert!(
            matches!(result, PlanetRepresentation::Impostor),
            "Extreme distance should use impostor, got {result:?}"
        );

        let result = select_planet_representation(100_000.0, &config);
        assert!(
            matches!(result, PlanetRepresentation::Geometry),
            "Close distance should use geometry, got {result:?}"
        );
    }

    #[test]
    fn test_impostor_texture_updates_on_view_change() {
        let impostor = ImpostorState {
            captured_view_dir: Vec3::Z,
            captured_sun_dir: Vec3::Y,
            update_threshold: 0.05,
            dirty: false,
        };

        // Same view direction: no update needed.
        assert!(
            !impostor.needs_update(Vec3::Z, Vec3::Y),
            "Same view direction should not need update"
        );

        // Slightly different view: no update needed (below threshold).
        let small_change = Vec3::new(0.01, 0.0, 1.0).normalize();
        assert!(
            !impostor.needs_update(small_change, Vec3::Y),
            "Small view change should not need update"
        );

        // Large view change: update needed.
        let large_change = Vec3::new(1.0, 0.0, 1.0).normalize(); // 45 degrees
        assert!(
            impostor.needs_update(large_change, Vec3::Y),
            "Large view change should trigger update"
        );

        // Sun direction change: update needed.
        let new_sun = Vec3::new(1.0, 1.0, 0.0).normalize();
        assert!(
            impostor.needs_update(Vec3::Z, new_sun),
            "Sun direction change should trigger update"
        );
    }

    #[test]
    fn test_impostor_correctly_sized_for_distance() {
        let planet_radius = 6_371_000.0; // Earth-like, meters

        let size_near = impostor_quad_size(planet_radius, planet_radius * 10.0);
        let size_far = impostor_quad_size(planet_radius, planet_radius * 100.0);
        let size_very_far = impostor_quad_size(planet_radius, planet_radius * 1000.0);

        assert!(
            size_near > size_far,
            "Near size ({size_near}) should be larger than far ({size_far})"
        );
        assert!(
            size_far > size_very_far,
            "Far size ({size_far}) should be larger than very far ({size_very_far})"
        );

        let expected_near =
            2.0 * (planet_radius / (planet_radius * 10.0)).asin().tan() * planet_radius * 10.0;
        assert!(
            ((size_near as f64) - expected_near).abs() / expected_near < 0.01,
            "Near size {size_near} should match expected {expected_near}"
        );
    }

    #[test]
    fn test_geometry_to_impostor_transition_is_smooth() {
        let config = ImpostorConfig::default();
        let start = config.impostor_distance - config.transition_band;
        let end = config.impostor_distance + config.transition_band * 2.0;
        let steps = 100;

        let mut prev_alpha = 0.0_f32;
        for i in 0..=steps {
            let distance = start + (end - start) * (i as f64 / steps as f64);
            let rep = select_planet_representation(distance, &config);
            let alpha = match rep {
                PlanetRepresentation::Geometry => 0.0,
                PlanetRepresentation::Blending { impostor_alpha } => impostor_alpha,
                PlanetRepresentation::Impostor => 1.0,
            };

            assert!(
                alpha >= prev_alpha - 1e-6,
                "Alpha decreased at distance {distance}: {prev_alpha} -> {alpha}"
            );
            prev_alpha = alpha;
        }
    }

    #[test]
    fn test_impostor_is_two_triangles() {
        let vertices = billboard_vertices(Vec3::ZERO, Vec3::X, Vec3::Y, 1.0);
        assert_eq!(vertices.len(), 4, "Impostor should have 4 vertices");
        assert_eq!(
            IMPOSTOR_INDICES.len(),
            6,
            "Impostor should have 6 indices (2 triangles)"
        );

        assert_eq!(IMPOSTOR_INDICES[0..3], [0, 1, 2]);
        assert_eq!(IMPOSTOR_INDICES[3..6], [0, 2, 3]);
    }

    #[test]
    fn test_impostor_state_starts_dirty() {
        let state = ImpostorState::new(0.05);
        assert!(state.dirty);
        assert!(state.needs_update(Vec3::Z, Vec3::Y));
    }

    #[test]
    fn test_impostor_state_mark_captured() {
        let mut state = ImpostorState::new(0.05);
        state.mark_captured(Vec3::X, Vec3::Y);
        assert!(!state.dirty);
        assert!(!state.needs_update(Vec3::X, Vec3::Y));
    }
}
