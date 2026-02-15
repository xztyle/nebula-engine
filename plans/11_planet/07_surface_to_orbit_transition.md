# Surface-to-Orbit Transition

## Problem

The engine has two rendering paths: voxel chunk meshes for the surface (stories 01-03) and a textured sphere for orbit (story 06). But there is no mechanism to transition between them. If the camera ascends from the surface and at some altitude the renderer abruptly switches from detailed voxels to a low-resolution sphere, the player sees a jarring visual pop. Additionally, f32 precision degrades as the camera moves away from the coordinate origin -- at 100 km altitude, positions 1 meter apart become indistinguishable in f32, causing mesh jittering and z-fighting. The LOD system (Epic 10) manages which chunks load at which detail level, but this story handles the visual blending, the chunk budget management during ascent, and the coordinate origin rebasing that keeps f32 precision intact at all altitudes.

## Solution

### Altitude-Based Rendering Mode

Define three altitude zones relative to the planet surface, with smooth blending between them:

```rust
/// Rendering mode based on camera altitude above the planet surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlanetRenderMode {
    /// Close to the surface: render voxel chunk meshes only.
    Surface,
    /// Transitioning: blend between voxel meshes and orbital sphere.
    Transition { blend_factor: f32 },
    /// Far from the surface: render orbital sphere only.
    Orbital,
}

pub struct TransitionConfig {
    /// Altitude (meters) below which only voxel rendering is used.
    pub surface_ceiling: f64,
    /// Altitude (meters) above which only orbital rendering is used.
    pub orbital_floor: f64,
}

impl Default for TransitionConfig {
    fn default() -> Self {
        Self {
            surface_ceiling: 50_000.0,  // 50 km
            orbital_floor: 200_000.0,   // 200 km
        }
    }
}

impl TransitionConfig {
    /// Determine the render mode for a given altitude.
    pub fn classify(&self, altitude: f64) -> PlanetRenderMode {
        if altitude <= self.surface_ceiling {
            PlanetRenderMode::Surface
        } else if altitude >= self.orbital_floor {
            PlanetRenderMode::Orbital
        } else {
            let t = ((altitude - self.surface_ceiling)
                / (self.orbital_floor - self.surface_ceiling)) as f32;
            PlanetRenderMode::Transition {
                blend_factor: smoothstep(0.0, 1.0, t),
            }
        }
    }
}
```

### Visual Blending

During the transition zone, both renderers run simultaneously. The voxel meshes fade out (alpha decreasing) while the orbital sphere fades in. The blend is applied via a per-frame uniform:

```rust
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TransitionUniform {
    /// 0.0 = fully surface (voxels opaque, sphere hidden)
    /// 1.0 = fully orbital (voxels hidden, sphere opaque)
    pub blend_factor: f32,
    pub _padding: [f32; 3],
}
```

The voxel pass multiplies fragment alpha by `1.0 - blend_factor`. The orbital pass multiplies fragment alpha by `blend_factor`. Both write to the same render target with alpha blending enabled during the transition.

### Chunk Budget Management

As the camera ascends, the LOD system loads fewer chunks at coarser detail. This story defines the chunk budget curve:

```rust
/// Compute the maximum number of active chunks based on altitude.
///
/// Near the surface, the budget is high (many detailed chunks).
/// At orbital altitude, the budget drops to zero (no voxel chunks needed).
pub fn chunk_budget_for_altitude(altitude: f64, config: &TransitionConfig) -> u32 {
    if altitude >= config.orbital_floor {
        return 0;
    }
    if altitude <= config.surface_ceiling {
        return 4096; // Maximum surface budget
    }

    let t = (altitude - config.surface_ceiling)
        / (config.orbital_floor - config.surface_ceiling);
    let budget = 4096.0 * (1.0 - t * t); // Quadratic falloff
    budget.max(0.0) as u32
}
```

The chunk manager respects this budget by unloading the most distant chunks when the budget decreases, and loading new chunks when the budget increases (on descent).

### Coordinate Origin Rebasing

To maintain f32 precision, the engine periodically shifts the coordinate origin to stay near the camera. This is especially critical during the transition when the camera is tens of kilometers from the planet surface:

```rust
use nebula_coords::WorldPosition;

/// Determines whether the coordinate origin should be updated.
///
/// If the camera has moved more than `threshold` millimeters from the current
/// origin, the origin is rebased to the camera's position and all local-space
/// positions are adjusted.
pub struct OriginManager {
    /// Current coordinate origin in world space.
    pub origin: WorldPosition,
    /// Distance threshold (mm) before rebasing. Default: 10 km = 10_000_000 mm.
    pub rebase_threshold: i128,
}

impl OriginManager {
    pub fn new() -> Self {
        Self {
            origin: WorldPosition::ZERO,
            rebase_threshold: 10_000_000_000, // 10 km in mm
        }
    }

    /// Check if the origin should be rebased and return the delta if so.
    pub fn update(&mut self, camera_world: &WorldPosition) -> Option<WorldPosition> {
        let dx = (camera_world.x - self.origin.x).abs();
        let dy = (camera_world.y - self.origin.y).abs();
        let dz = (camera_world.z - self.origin.z).abs();

        if dx > self.rebase_threshold
            || dy > self.rebase_threshold
            || dz > self.rebase_threshold
        {
            let old_origin = self.origin;
            self.origin = *camera_world;
            Some(WorldPosition {
                x: camera_world.x - old_origin.x,
                y: camera_world.y - old_origin.y,
                z: camera_world.z - old_origin.z,
            })
        } else {
            None
        }
    }
}
```

When the origin shifts, all GPU-side positions (chunk meshes, camera uniform) are recomputed relative to the new origin. Existing mesh buffers are invalidated and re-uploaded with adjusted positions.

### Ascent/Descent Pipeline

The complete per-frame pipeline during vertical movement:

```rust
pub fn update_planet_rendering(
    camera_world: &WorldPosition,
    planet: &mut PlanetState,
    origin: &mut OriginManager,
    config: &TransitionConfig,
) -> PlanetRenderMode {
    // 1. Rebase origin if needed.
    if let Some(_delta) = origin.update(camera_world) {
        planet.invalidate_local_positions();
    }

    // 2. Compute altitude above the surface.
    let altitude = planet.altitude_of(camera_world);

    // 3. Determine render mode.
    let mode = config.classify(altitude);

    // 4. Adjust chunk budget.
    let budget = chunk_budget_for_altitude(altitude, config);
    planet.set_chunk_budget(budget);

    // 5. Update LOD quadtrees (coarser chunks at higher altitude).
    planet.update_lod(camera_world);

    mode
}
```

## Outcome

The `nebula-planet` crate exports `PlanetRenderMode`, `TransitionConfig`, `OriginManager`, `chunk_budget_for_altitude()`, and `update_planet_rendering()`. The camera can ascend from the surface to orbit without any visual discontinuity. During the transition zone, voxel meshes fade out while the orbital sphere fades in. The chunk budget decreases with altitude, keeping memory and GPU usage bounded. The coordinate origin automatically rebases to maintain f32 precision. The transition is fully reversible: descending from orbit smoothly loads voxel chunks and fades the orbital sphere away.

## Demo Integration

**Demo crate:** `nebula-demo`

Flying upward from the surface, voxel terrain fades out and the orbital sphere fades in. The transition is seamless with no visual discontinuity. Descending reverses the process.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | `0.29` | Camera position math, matrix updates |
| `bytemuck` | `1.21` | Transition uniform serialization |

Internal dependencies: `nebula-coords` (WorldPosition, coordinate origin), `nebula-lod` (quadtree, LOD selection), `nebula-voxel` (chunk manager), `nebula-render` (pipeline, blend state). Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_coords::WorldPosition;

    #[test]
    fn test_no_visual_discontinuity_during_ascent() {
        let config = TransitionConfig::default();

        // Sample the blend factor at many altitudes during ascent.
        let mut prev_blend = 0.0_f32;
        let steps = 1000;
        for i in 0..=steps {
            let alt = i as f64 / steps as f64 * 300_000.0; // 0 to 300 km
            let mode = config.classify(alt);
            let blend = match mode {
                PlanetRenderMode::Surface => 0.0,
                PlanetRenderMode::Transition { blend_factor } => blend_factor,
                PlanetRenderMode::Orbital => 1.0,
            };

            // Blend should only increase during ascent (monotonic).
            assert!(
                blend >= prev_blend - 1e-6,
                "Blend decreased at altitude {alt}m: {prev_blend} -> {blend}"
            );

            // No large jumps.
            let delta = (blend - prev_blend).abs();
            assert!(
                delta < 0.05,
                "Blend jumped by {delta} at altitude {alt}m"
            );

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
    fn test_coordinate_origin_updates() {
        let mut origin = OriginManager::new();
        let camera_near = WorldPosition { x: 1_000_000, y: 1_000_000, z: 1_000_000 };

        // Small movement: no rebase.
        assert!(
            origin.update(&camera_near).is_none(),
            "Small movement should not trigger rebase"
        );

        // Large movement: should trigger rebase.
        let camera_far = WorldPosition {
            x: 100_000_000_000,
            y: 0,
            z: 0,
        };
        let delta = origin.update(&camera_far);
        assert!(
            delta.is_some(),
            "Large movement should trigger rebase"
        );
        assert_eq!(
            origin.origin, camera_far,
            "Origin should be updated to camera position"
        );
    }

    #[test]
    fn test_frame_rate_stable_during_transition() {
        // Verify that the chunk budget curve is smooth (no sudden budget changes).
        let config = TransitionConfig::default();
        let altitude_step = 100.0; // 100m steps

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

        // At 100m steps, the budget should not change by more than ~50 chunks per step.
        assert!(
            max_delta < 50,
            "Chunk budget changed by {max_delta} in a single 100m step — too abrupt"
        );
    }

    #[test]
    fn test_transition_is_reversible() {
        let config = TransitionConfig::default();

        // Ascend.
        let mode_at_100km = config.classify(100_000.0);
        // Descend back.
        let mode_at_10km = config.classify(10_000.0);

        assert!(
            matches!(mode_at_100km, PlanetRenderMode::Transition { .. }),
            "100 km should be in transition zone"
        );
        assert_eq!(
            mode_at_10km,
            PlanetRenderMode::Surface,
            "10 km should be back in surface mode"
        );

        // Blend factor at the same altitude should be identical regardless of direction.
        let ascending = config.classify(125_000.0);
        let descending = config.classify(125_000.0);
        assert_eq!(
            ascending, descending,
            "Transition should be symmetric: ascending={ascending:?}, descending={descending:?}"
        );
    }
}
```
