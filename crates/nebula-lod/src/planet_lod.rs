//! Planet-to-space LOD: selects rendering strategy based on camera distance from a planet.
//!
//! Transitions smoothly from voxel terrain (surface) → hybrid → geometric sphere → impostor
//! as the camera pulls away from a planet into space.

use nebula_math::WorldPosition;

/// The rendering strategy used for a planet at the current camera distance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanetRenderMode {
    /// Full voxel rendering with per-chunk LOD (stories 01-07).
    /// Used when the camera is on or near the surface.
    VoxelTerrain,
    /// Coarse voxel terrain blended with a geometric sphere underlay.
    /// Used during the transition from surface to orbital view.
    HybridTerrainSphere,
    /// A tessellated sphere mesh with a procedural heightmap displacement
    /// and baked terrain color texture. No individual voxels.
    GeometricSphere,
    /// A pre-rendered billboard (impostor) of the planet.
    /// Used at extreme distances where the planet is only a few pixels.
    Impostor,
}

/// Configuration for planet-to-space LOD transitions.
#[derive(Clone, Debug)]
pub struct PlanetLodConfig {
    /// Planet radius in world units (meters, stored as f64 for precision).
    pub radius: f64,

    /// Below this altitude (as fraction of radius), use full voxel rendering.
    /// Default: 0.01 (1% of radius, e.g., 64 km on a 6400 km planet).
    pub voxel_max_altitude: f64,

    /// Between voxel_max and this altitude, use hybrid rendering.
    /// Default: 0.05 (5% of radius, e.g., 320 km).
    pub hybrid_max_altitude: f64,

    /// Between hybrid_max and this distance (from planet center, as fraction
    /// of radius), use geometric sphere.
    /// Default: 10.0 (10× radius, e.g., 64,000 km).
    pub sphere_max_distance: f64,

    /// Beyond sphere_max_distance, use impostor billboard.
    /// Default: 100.0 (100× radius).
    pub impostor_distance: f64,
}

impl Default for PlanetLodConfig {
    fn default() -> Self {
        Self {
            radius: 6_400_000.0,
            voxel_max_altitude: 0.01,
            hybrid_max_altitude: 0.05,
            sphere_max_distance: 10.0,
            impostor_distance: 100.0,
        }
    }
}

/// Determines the rendering mode for a planet based on camera distance.
pub struct PlanetLodSelector {
    config: PlanetLodConfig,
}

impl PlanetLodSelector {
    /// Create a new selector with the given configuration.
    pub fn new(config: PlanetLodConfig) -> Self {
        Self { config }
    }

    /// Return a reference to the current configuration.
    pub fn config(&self) -> &PlanetLodConfig {
        &self.config
    }

    /// Select the rendering mode based on camera altitude above the surface.
    /// Also returns a blend factor (0.0 to 1.0) for smooth transitions.
    pub fn select_mode(&self, camera_altitude: f64) -> (PlanetRenderMode, f32) {
        let r = self.config.radius;

        let voxel_max = r * self.config.voxel_max_altitude;
        let hybrid_max = r * self.config.hybrid_max_altitude;
        let sphere_max = r * self.config.sphere_max_distance;

        if camera_altitude < voxel_max {
            (PlanetRenderMode::VoxelTerrain, 0.0)
        } else if camera_altitude < hybrid_max {
            // Blend factor: 0 at voxel_max, 1 at hybrid_max
            let t = ((camera_altitude - voxel_max) / (hybrid_max - voxel_max)) as f32;
            (PlanetRenderMode::HybridTerrainSphere, t.clamp(0.0, 1.0))
        } else if camera_altitude < sphere_max {
            (PlanetRenderMode::GeometricSphere, 0.0)
        } else {
            (PlanetRenderMode::Impostor, 0.0)
        }
    }

    /// Calculate camera altitude above the planet surface.
    /// Uses 128-bit world coordinates for precision.
    pub fn camera_altitude(
        camera_pos: &WorldPosition,
        planet_center: &WorldPosition,
        planet_radius: f64,
    ) -> f64 {
        let dx = (camera_pos.x - planet_center.x) as f64;
        let dy = (camera_pos.y - planet_center.y) as f64;
        let dz = (camera_pos.z - planet_center.z) as f64;
        let distance_to_center = (dx * dx + dy * dy + dz * dz).sqrt();
        (distance_to_center - planet_radius).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_selector() -> PlanetLodSelector {
        PlanetLodSelector::new(PlanetLodConfig::default())
    }

    fn earth_radius() -> f64 {
        6_400_000.0
    }

    /// Camera on the planet surface should use voxel terrain rendering.
    #[test]
    fn test_camera_on_surface_uses_voxel() {
        let selector = default_selector();
        let (mode, _blend) = selector.select_mode(0.0);
        assert_eq!(mode, PlanetRenderMode::VoxelTerrain);

        // Also at low altitude (e.g., 1 km)
        let (mode, _blend) = selector.select_mode(1_000.0);
        assert_eq!(mode, PlanetRenderMode::VoxelTerrain);
    }

    /// Camera in low orbit should use geometric sphere rendering.
    #[test]
    fn test_camera_in_orbit_uses_sphere() {
        let selector = default_selector();
        let r = earth_radius();
        let altitude = r * 0.10;
        let (mode, _blend) = selector.select_mode(altitude);
        assert_eq!(mode, PlanetRenderMode::GeometricSphere);
    }

    /// Camera at extreme distance should use impostor billboard.
    #[test]
    fn test_extreme_distance_uses_impostor() {
        let selector = default_selector();
        let r = earth_radius();
        let altitude = r * 20.0;
        let (mode, _blend) = selector.select_mode(altitude);
        assert_eq!(mode, PlanetRenderMode::Impostor);
    }

    /// Transition distances should be configurable.
    #[test]
    fn test_transition_distances_configurable() {
        let config = PlanetLodConfig {
            radius: 1_000_000.0,
            voxel_max_altitude: 0.02,
            hybrid_max_altitude: 0.1,
            sphere_max_distance: 5.0,
            impostor_distance: 50.0,
        };
        let selector = PlanetLodSelector::new(config);

        let (mode, _) = selector.select_mode(10_000.0);
        assert_eq!(mode, PlanetRenderMode::VoxelTerrain);

        let (mode, _) = selector.select_mode(50_000.0);
        assert_eq!(mode, PlanetRenderMode::HybridTerrainSphere);
    }

    /// Transition from voxel to hybrid should produce a blend factor
    /// that interpolates smoothly from 0 to 1.
    #[test]
    fn test_no_visual_pop_during_transitions() {
        let selector = default_selector();
        let r = earth_radius();

        let voxel_max = r * 0.01;
        let hybrid_max = r * 0.05;

        let (mode, blend) = selector.select_mode(voxel_max + 1.0);
        assert_eq!(mode, PlanetRenderMode::HybridTerrainSphere);
        assert!(
            blend < 0.01,
            "blend should be near 0 at start of hybrid zone, got {blend}"
        );

        let midpoint = (voxel_max + hybrid_max) / 2.0;
        let (_mode, blend) = selector.select_mode(midpoint);
        assert!(
            (blend - 0.5).abs() < 0.05,
            "blend should be near 0.5 at midpoint, got {blend}"
        );

        let (mode, blend) = selector.select_mode(hybrid_max - 1.0);
        assert_eq!(mode, PlanetRenderMode::HybridTerrainSphere);
        assert!(
            blend > 0.99,
            "blend should be near 1 at end of hybrid zone, got {blend}"
        );
    }

    /// Camera altitude calculation from world positions.
    #[test]
    fn test_camera_altitude_calculation() {
        let planet_center = WorldPosition { x: 0, y: 0, z: 0 };
        let radius = 6_400_000.0;

        // Camera directly above at surface + 1000m
        let camera = WorldPosition {
            x: 0,
            y: (radius as i128) + 1_000,
            z: 0,
        };
        let alt = PlanetLodSelector::camera_altitude(&camera, &planet_center, radius);
        assert!((alt - 1_000.0).abs() < 1.0, "expected ~1000, got {alt}");
    }

    /// Camera inside the planet should clamp altitude to 0.
    #[test]
    fn test_camera_inside_planet_clamps_to_zero() {
        let planet_center = WorldPosition { x: 0, y: 0, z: 0 };
        let radius = 6_400_000.0;
        let camera = WorldPosition { x: 0, y: 0, z: 0 };
        let alt = PlanetLodSelector::camera_altitude(&camera, &planet_center, radius);
        assert_eq!(alt, 0.0);
    }
}
