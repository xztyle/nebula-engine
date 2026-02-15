# Planet-to-Space LOD

## Problem

The engine must render planets at scales ranging from individual voxels on the surface (1-meter resolution) to entire planets seen from interplanetary distances (thousands of kilometers away). The voxel LOD system (stories 01-07) handles surface-level detail through LOD levels 0-4, but at extreme distances the planet subtends only a few pixels on screen and the quadtree would need millions of coarse leaf nodes to cover the entire planet surface — even at the coarsest voxel LOD. Beyond a certain distance, voxel representation becomes both wasteful and visually inferior to a simple geometric sphere with a texture. The engine needs a multi-stage rendering pipeline that seamlessly transitions from voxel terrain to simplified geometric representations as the camera pulls away from the planet surface into orbit and beyond.

## Solution

Implement a planet rendering mode selector in the `nebula_lod` crate that chooses between four rendering strategies based on camera distance from the planet surface. The transitions are smooth (no hard pops) and the distance thresholds are configurable per planet.

### Rendering Modes

```rust
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
```

### Distance Thresholds

Distances are measured from the camera to the planet surface (not center). Thresholds are expressed as multiples of the planet's radius for scale-independence:

```rust
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
    /// Default: 10.0 (10x radius, e.g., 64,000 km).
    pub sphere_max_distance: f64,

    /// Beyond sphere_max_distance, use impostor billboard.
    /// Default: 100.0 (100x radius).
    pub impostor_distance: f64,
}

impl Default for PlanetLodConfig {
    fn default() -> Self {
        Self {
            radius: 6_400_000.0, // Earth-like radius in meters
            voxel_max_altitude: 0.01,
            hybrid_max_altitude: 0.05,
            sphere_max_distance: 10.0,
            impostor_distance: 100.0,
        }
    }
}
```

### Mode Selector

```rust
/// Determines the rendering mode for a planet based on camera distance.
pub struct PlanetLodSelector {
    config: PlanetLodConfig,
}

impl PlanetLodSelector {
    pub fn new(config: PlanetLodConfig) -> Self {
        Self { config }
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
```

### Hybrid Mode

The hybrid mode renders both the voxel terrain (at coarse LOD levels) and a geometric sphere underneath. The sphere acts as a "fill" for areas where the voxel terrain is too coarse to cover completely. The blend factor controls how much the sphere shows through:

- At blend 0.0: voxel terrain only (just entered hybrid zone).
- At blend 0.5: voxel terrain is partially transparent, sphere is visible beneath.
- At blend 1.0: fully transitioned to sphere-only rendering.

### Impostor Generation

At extreme distances, the planet is rendered as a billboard quad facing the camera with a pre-rendered texture. The impostor is regenerated when the camera moves significantly relative to the planet (changing the visible hemisphere). The impostor texture includes atmosphere glow and is rendered at a resolution proportional to the planet's screen-space size.

## Outcome

The `nebula_lod` crate exports `PlanetRenderMode`, `PlanetLodConfig`, and `PlanetLodSelector`. Each frame, the renderer queries the selector to determine which rendering strategy to use for each visible planet. The transition between modes is smooth (blended over a distance range, not a hard cut). Running `cargo test -p nebula_lod` passes all planet-to-space LOD tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Flying away from a planet, the voxel terrain smoothly transitions to a textured sphere and then to a point sprite. The transition is gradual with no visible pop.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_math` | workspace | `WorldPosition` (128-bit), distance calculations |
| `nebula_rendering` | workspace | Render pass management for mode switching |
| `nebula_lod` | workspace (self) | Core LOD types |

No external crates required for the mode selection logic. The crate uses Rust edition 2024.

## Unit Tests

```rust
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
        // Altitude = 10% of radius = 640 km (well above hybrid threshold of 5%)
        let altitude = r * 0.10;
        let (mode, _blend) = selector.select_mode(altitude);
        assert_eq!(mode, PlanetRenderMode::GeometricSphere);
    }

    /// Camera at extreme distance should use impostor billboard.
    #[test]
    fn test_extreme_distance_uses_impostor() {
        let selector = default_selector();
        let r = earth_radius();
        // Distance = 20x radius from surface
        let altitude = r * 20.0;
        let (mode, _blend) = selector.select_mode(altitude);
        assert_eq!(mode, PlanetRenderMode::Impostor);
    }

    /// Transition distances should be configurable.
    #[test]
    fn test_transition_distances_configurable() {
        let config = PlanetLodConfig {
            radius: 1_000_000.0,
            voxel_max_altitude: 0.02, // 2% = 20 km
            hybrid_max_altitude: 0.1, // 10% = 100 km
            sphere_max_distance: 5.0, // 5x radius
            impostor_distance: 50.0,
        };
        let selector = PlanetLodSelector::new(config);

        // At 10 km altitude (below 2% of 1M = 20 km), should be voxel
        let (mode, _) = selector.select_mode(10_000.0);
        assert_eq!(mode, PlanetRenderMode::VoxelTerrain);

        // At 50 km altitude (between 2% and 10%), should be hybrid
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

        // At exactly the voxel boundary, blend should be near 0
        let (mode, blend) = selector.select_mode(voxel_max + 1.0);
        assert_eq!(mode, PlanetRenderMode::HybridTerrainSphere);
        assert!(blend < 0.01, "blend should be near 0 at start of hybrid zone, got {blend}");

        // At the midpoint of the hybrid zone, blend should be near 0.5
        let midpoint = (voxel_max + hybrid_max) / 2.0;
        let (_mode, blend) = selector.select_mode(midpoint);
        assert!((blend - 0.5).abs() < 0.05, "blend should be near 0.5 at midpoint, got {blend}");

        // Near the end of the hybrid zone, blend should be near 1
        let (mode, blend) = selector.select_mode(hybrid_max - 1.0);
        assert_eq!(mode, PlanetRenderMode::HybridTerrainSphere);
        assert!(blend > 0.99, "blend should be near 1 at end of hybrid zone, got {blend}");
    }
}
```
