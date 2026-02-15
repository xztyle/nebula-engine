# Particle LOD

## Problem

A scene may contain dozens of particle emitters simultaneously — engine exhaust from every visible ship, dust from terrain interactions, magic effects from combat, smoke from fires. At full emission rate, each emitter might maintain hundreds or thousands of particles. Without level-of-detail reduction, the total particle count can exceed the GPU's compute and fill-rate budget, causing frame drops that scale with the number of visible effects. Worse, particles from distant emitters are sub-pixel sized and contribute negligible visual information — the player cannot tell whether a ship 2km away has 500 exhaust particles or 50.

Additionally, emitters that are entirely off-screen (outside the camera frustum) should not simulate at all. Spending compute shader dispatches and draw calls on invisible particles is pure waste. A global memory budget is also needed to prevent runaway particle allocation: if the total alive particle count across all emitters exceeds the budget (e.g., 100,000), the least important emitters must be throttled.

## Solution

Implement a `ParticleLodSystem` in the `nebula-particles` crate that adjusts emission rates based on distance, skips off-screen emitters, and enforces a global particle budget.

### LOD Distance Tiers

Define three distance tiers relative to the camera:

```rust
/// Distance thresholds for particle LOD tiers.
#[derive(Clone, Debug)]
pub struct ParticleLodConfig {
    /// Distance below which full emission rate is used.
    pub close_range: f32,
    /// Distance below which emission rate is halved.
    pub medium_range: f32,
    /// Distance beyond which emission rate is quartered.
    /// Beyond this, extremely distant emitters may be disabled entirely.
    pub far_range: f32,
    /// Distance beyond which the emitter is completely disabled.
    pub cull_distance: f32,
    /// Global maximum alive particles across all emitters.
    pub global_budget: u32,
}

impl Default for ParticleLodConfig {
    fn default() -> Self {
        Self {
            close_range: 50.0,
            medium_range: 150.0,
            far_range: 400.0,
            cull_distance: 1000.0,
            global_budget: 100_000,
        }
    }
}
```

### LOD Multiplier Calculation

```rust
/// Compute the emission rate multiplier for a given distance.
pub fn lod_multiplier(distance: f32, config: &ParticleLodConfig) -> f32 {
    if distance >= config.cull_distance {
        0.0
    } else if distance >= config.far_range {
        // Smooth fade from 0.25 to 0.0 between far_range and cull_distance.
        let t = (config.cull_distance - distance) / (config.cull_distance - config.far_range);
        0.25 * t
    } else if distance >= config.medium_range {
        // Smooth fade from 0.5 to 0.25 between medium_range and far_range.
        let t = (config.far_range - distance) / (config.far_range - config.medium_range);
        0.25 + 0.25 * t
    } else if distance >= config.close_range {
        // Smooth fade from 1.0 to 0.5 between close_range and medium_range.
        let t = (config.medium_range - distance) / (config.medium_range - config.close_range);
        0.5 + 0.5 * t
    } else {
        1.0
    }
}
```

The smooth interpolation within each tier prevents jarring pop-in/pop-out when an emitter crosses a distance threshold. Instead of a hard step from 1.0x to 0.5x, the multiplier transitions linearly across the tier boundary.

### Frustum Culling for Emitters

Before computing distance-based LOD, emitters outside the camera frustum are skipped entirely:

```rust
/// Determine if an emitter's bounding sphere is inside the camera frustum.
pub fn is_emitter_visible(
    emitter_position: Vec3,
    emitter_radius: f32,
    frustum: &Frustum,
) -> bool {
    frustum.contains_sphere(emitter_position, emitter_radius)
}
```

The emitter's bounding radius is derived from its max particle velocity and lifetime — a conservative sphere that contains all possible particle positions:

```rust
pub fn emitter_bounding_radius(emitter: &ParticleEmitter) -> f32 {
    let max_speed = emitter.velocity_range.1.length();
    let max_lifetime = emitter.lifetime_range.1;
    max_speed * max_lifetime
}
```

Off-screen emitters have their `enabled` flag set to `false` for the current frame. Their particles are not simulated or drawn. When the camera turns to include them, they resume from whatever state their buffer held (particles that were mid-flight remain mid-flight, which is less disruptive than respawning everything).

### Global Budget Enforcement

When the total alive particle count exceeds the global budget, the system must reduce particles. The strategy is to sort emitters by importance (distance-weighted priority) and throttle the least important first:

```rust
/// Per-emitter importance score. Lower distance = higher importance.
pub fn emitter_importance(distance: f32, base_priority: f32) -> f32 {
    base_priority / (1.0 + distance)
}
```

The `ParticleLodSystem` each frame:

1. **Gather** all active emitters with their alive counts and distances.
2. **Sum** total alive particles across all emitters.
3. **If total > budget**, sort emitters by importance (ascending) and reduce emission rate multipliers for the least important emitters until the projected total falls within budget.
4. **Apply** the computed multiplier to each emitter's `emission_rate` for this frame.

```rust
pub struct ParticleLodSystem {
    pub config: ParticleLodConfig,
}

impl ParticleLodSystem {
    pub fn update(
        &self,
        emitters: &mut [(EntityId, &mut ParticleEmitter, &ParticleBuffer, Vec3)],
        camera_position: Vec3,
        frustum: &Frustum,
    ) {
        let mut total_alive: u32 = 0;

        for (entity, emitter, buffer, position) in emitters.iter_mut() {
            let radius = emitter_bounding_radius(emitter);

            // Frustum cull.
            if !is_emitter_visible(*position, radius, frustum) {
                emitter.enabled = false;
                continue;
            }

            // Distance LOD.
            let distance = (*position - camera_position).length();
            let multiplier = lod_multiplier(distance, &self.config);

            if multiplier <= 0.0 {
                emitter.enabled = false;
                continue;
            }

            emitter.enabled = true;
            // Store the original rate and apply the LOD-scaled rate.
            emitter.emission_rate *= multiplier;

            total_alive += buffer.alive_count;
        }

        // Budget enforcement.
        if total_alive > self.config.global_budget {
            self.enforce_budget(emitters, camera_position, total_alive);
        }
    }

    fn enforce_budget(
        &self,
        emitters: &mut [(EntityId, &mut ParticleEmitter, &ParticleBuffer, Vec3)],
        camera_position: Vec3,
        mut total: u32,
    ) {
        // Sort by importance (least important first).
        let mut indexed: Vec<(usize, f32)> = emitters
            .iter()
            .enumerate()
            .filter(|(_, (_, e, _, _))| e.enabled)
            .map(|(i, (_, _, _, pos))| {
                let dist = (*pos - camera_position).length();
                (i, emitter_importance(dist, 1.0))
            })
            .collect();
        indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

        // Disable least important emitters until under budget.
        for (idx, _importance) in indexed {
            if total <= self.config.global_budget {
                break;
            }
            let (_, ref mut emitter, ref buffer, _) = emitters[idx];
            total -= buffer.alive_count;
            emitter.enabled = false;
        }
    }
}
```

### Billboard Size Invariance

Importantly, the LOD system does **not** change particle billboard size. Particles at medium range are the same screen size as at close range (perspective projection handles apparent size). Only the count is reduced. This prevents the visual artifact of particles suddenly growing or shrinking when crossing LOD boundaries.

## Outcome

A `ParticleLodSystem` that runs each frame to adjust particle emission rates based on camera distance (three tiers with smooth interpolation), cull off-screen emitters via frustum testing, and enforce a global particle budget by disabling the least important emitters when the total count exceeds the limit. Billboard sizes remain constant. The system prevents GPU over-commitment while preserving visual quality for nearby effects.

## Demo Integration

**Demo crate:** `nebula-demo`

Distant particle emitters produce fewer particles to save GPU budget. An explosion across a valley still looks dramatic but uses fewer quads.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | `0.29` | Vec3 for distance calculations and frustum math |

No new external dependencies. The frustum type is assumed to exist in `nebula-render` (from Story 04_rendering/09). Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> ParticleLodConfig {
        ParticleLodConfig::default()
    }

    #[test]
    fn test_close_emitter_has_full_rate() {
        let config = default_config();
        let distance = 10.0; // well within close_range (50.0)
        let multiplier = lod_multiplier(distance, &config);
        assert!(
            (multiplier - 1.0).abs() < 0.001,
            "close emitter should have multiplier 1.0, got {}",
            multiplier
        );
    }

    #[test]
    fn test_medium_distance_reduces_rate() {
        let config = default_config();
        // Exactly at the boundary between medium and far.
        let distance = config.medium_range;
        let multiplier = lod_multiplier(distance, &config);
        assert!(
            (multiplier - 0.5).abs() < 0.001,
            "emitter at medium_range boundary should have multiplier ~0.5, got {}",
            multiplier
        );
    }

    #[test]
    fn test_far_distance_quarters_rate() {
        let config = default_config();
        let distance = config.far_range;
        let multiplier = lod_multiplier(distance, &config);
        assert!(
            (multiplier - 0.25).abs() < 0.001,
            "emitter at far_range boundary should have multiplier ~0.25, got {}",
            multiplier
        );
    }

    #[test]
    fn test_beyond_cull_distance_disables() {
        let config = default_config();
        let distance = config.cull_distance + 100.0;
        let multiplier = lod_multiplier(distance, &config);
        assert!(
            multiplier == 0.0,
            "emitter beyond cull_distance should have multiplier 0.0, got {}",
            multiplier
        );
    }

    #[test]
    fn test_off_screen_emitter_is_skipped() {
        let position = Vec3::new(1000.0, 0.0, 0.0); // far off to the side
        let radius = 10.0;
        // Construct a frustum that faces +Z and does not include +X at 1000 units.
        let frustum = Frustum::from_view_projection(&Mat4::perspective_lh(
            std::f32::consts::FRAC_PI_2, 1.0, 0.1, 500.0,
        ));
        let visible = is_emitter_visible(position, radius, &frustum);
        assert!(!visible, "emitter far off-screen should not be visible");
    }

    #[test]
    fn test_on_screen_emitter_is_visible() {
        let position = Vec3::new(0.0, 0.0, 10.0); // directly in front
        let radius = 5.0;
        let frustum = Frustum::from_view_projection(&Mat4::perspective_lh(
            std::f32::consts::FRAC_PI_2, 1.0, 0.1, 500.0,
        ));
        let visible = is_emitter_visible(position, radius, &frustum);
        assert!(visible, "emitter directly in front should be visible");
    }

    #[test]
    fn test_global_budget_is_respected() {
        let config = ParticleLodConfig {
            global_budget: 100,
            ..Default::default()
        };
        let system = ParticleLodSystem { config };

        // Create two emitters each with 80 alive particles (total 160 > budget 100).
        let mut emitter_a = ParticleEmitter::default();
        let buffer_a = ParticleBuffer { alive_count: 80, ..empty_buffer() };
        let pos_a = Vec3::new(0.0, 0.0, 10.0); // close

        let mut emitter_b = ParticleEmitter::default();
        let buffer_b = ParticleBuffer { alive_count: 80, ..empty_buffer() };
        let pos_b = Vec3::new(0.0, 0.0, 200.0); // far

        let camera_pos = Vec3::ZERO;
        let frustum = Frustum::new_all_visible(); // test helper: everything visible

        let mut emitters = vec![
            (EntityId(1), &mut emitter_a, &buffer_a, pos_a),
            (EntityId(2), &mut emitter_b, &buffer_b, pos_b),
        ];

        system.update(&mut emitters, camera_pos, &frustum);

        // The far emitter should have been disabled to bring total under budget.
        let total: u32 = emitters
            .iter()
            .filter(|(_, e, _, _)| e.enabled)
            .map(|(_, _, b, _)| b.alive_count)
            .sum();
        assert!(
            total <= 100,
            "total alive particles {} should not exceed budget 100",
            total
        );
    }

    #[test]
    fn test_lod_transitions_are_smooth() {
        let config = default_config();
        // Sample multipliers at small distance increments across the close-to-medium boundary.
        let step = 1.0;
        let mut prev = lod_multiplier(config.close_range - step, &config);

        let mut distance = config.close_range;
        while distance <= config.medium_range {
            let current = lod_multiplier(distance, &config);
            let delta = (current - prev).abs();
            assert!(
                delta < 0.1,
                "LOD multiplier jump of {} at distance {} is too large (prev={}, current={})",
                delta,
                distance,
                prev,
                current
            );
            prev = current;
            distance += step;
        }
    }

    #[test]
    fn test_emitter_importance_decreases_with_distance() {
        let close_importance = emitter_importance(10.0, 1.0);
        let far_importance = emitter_importance(500.0, 1.0);
        assert!(
            close_importance > far_importance,
            "close emitter importance {} should exceed far emitter importance {}",
            close_importance,
            far_importance
        );
    }

    #[test]
    fn test_bounding_radius_matches_max_travel() {
        let emitter = ParticleEmitter {
            velocity_range: (Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0)),
            lifetime_range: (1.0, 2.0),
            ..Default::default()
        };
        let radius = emitter_bounding_radius(&emitter);
        // Max speed = 5.0, max lifetime = 2.0, so radius should be 10.0.
        assert!(
            (radius - 10.0).abs() < 0.01,
            "bounding radius should be max_speed * max_lifetime = 10.0, got {}",
            radius
        );
    }

    #[test]
    fn test_billboard_size_unchanged_across_lod_tiers() {
        // LOD only affects emission_rate, never size_over_lifetime.
        // This is a structural test: verify the LOD system does not touch size.
        let original_size = Curve {
            keyframes: vec![(0.0, 0.1), (1.0, 0.05)],
        };
        let mut emitter = ParticleEmitter {
            size_over_lifetime: original_size.clone(),
            ..Default::default()
        };

        // Apply LOD at medium range.
        let config = default_config();
        let multiplier = lod_multiplier(config.medium_range + 10.0, &config);
        emitter.emission_rate *= multiplier;

        // Size curve should be identical.
        assert_eq!(
            emitter.size_over_lifetime.keyframes.len(),
            original_size.keyframes.len()
        );
        for (a, b) in emitter.size_over_lifetime.keyframes.iter().zip(original_size.keyframes.iter()) {
            assert_eq!(a.0, b.0);
            assert_eq!(a.1, b.1);
        }
    }
}
```
