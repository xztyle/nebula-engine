# Multi-Source Gravity Blending

## Problem

In a solar system with planets and moons, an entity traveling between two celestial bodies experiences gravity from both simultaneously. The engine must decide how to combine multiple gravity fields. Three options exist: (A) use only the dominant (nearest or strongest) source, which is simple but creates a discontinuous "flip" at the boundary; (B) vector-sum all sources, which is physically realistic but means every source must be evaluated; (C) smooth blend with an artificial transition zone, which is a gameplay compromise. The choice affects whether Lagrange points (equilibrium points between two bodies) emerge naturally, whether gravity transitions feel smooth to the player, and whether the "up" direction for the camera is well-defined at all points in space.

## Solution

### Approach: Vector Sum (Option B)

The engine uses the vector sum of all contributing gravity sources. This is the physically correct approach and, critically, it produces Lagrange points as emergent behavior rather than requiring them to be hand-placed. The `compute_gravity` function from story 02 already implements vector summation — this story addresses the implications, edge cases, and the concept of a "dominant source" for camera and gameplay systems.

### Why Vector Sum Over Alternatives

- **Option A (dominant only)**: Creates a discontinuous gravity flip at the sphere-of-influence boundary. A player crossing the boundary between a planet and its moon would experience an instant reversal of "up" — physically wrong and disorienting.
- **Option C (smooth blend)**: Requires an artificial transition zone width to be tuned per pair of bodies. With N bodies, the number of pairwise zones grows quadratically. It also suppresses Lagrange points.
- **Option B (vector sum)**: Continuous everywhere. No tuning parameters. Lagrange points emerge naturally. The only cost is evaluating all sources within influence range, which the `GravitySourceRegistry` already filters efficiently.

### Dominant Source Determination

While the physics system uses the full vector sum for force application, several gameplay systems need to know the "dominant" gravity source — the one body whose gravity most strongly influences the entity:

```rust
/// Identifies the dominant gravity source for an entity.
/// Used by the camera (to define "up"), the HUD (to show which body
/// the player is bound to), and the orientation alignment system.
#[derive(Component, Debug, Clone, Copy)]
pub struct DominantGravitySource {
    /// Entity ID of the dominant source.
    pub entity: Entity,
    /// Magnitude of gravity from this source alone, in m/s².
    pub magnitude: f32,
    /// The fraction of total gravity contributed by this source (0.0 to 1.0).
    /// Near a single planet, this is ~1.0. At a Lagrange point, it may be ~0.5.
    pub dominance: f32,
}

/// Determine which gravity source dominates at the entity's position.
pub fn find_dominant_source(
    entity_pos: &WorldPos,
    registry: &GravitySourceRegistry,
) -> Option<DominantGravitySource> {
    let affecting = registry.sources_affecting(entity_pos);
    if affecting.is_empty() {
        return None;
    }

    let mut best_entity = Entity::PLACEHOLDER;
    let mut best_magnitude: f64 = 0.0;
    let mut total_magnitude: f64 = 0.0;

    for entry in &affecting {
        if let Some((_dir, mag)) = compute_gravity_from_source(
            entity_pos, &entry.position, &entry.source,
        ) {
            total_magnitude += mag;
            if mag > best_magnitude {
                best_magnitude = mag;
                best_entity = entry.entity;
            }
        }
    }

    if total_magnitude < 1e-9 {
        return None;
    }

    Some(DominantGravitySource {
        entity: best_entity,
        magnitude: best_magnitude as f32,
        dominance: (best_magnitude / total_magnitude) as f32,
    })
}
```

The `dominance` field (0.0 to 1.0) tells downstream systems how "confident" the dominant source assignment is. Near a planet's surface, dominance is ~1.0. Midway between two equal bodies, dominance drops to ~0.5. This can drive smooth camera transitions: when dominance drops below a threshold, the camera slows its "up" reorientation to avoid jarring flips.

### Lagrange Point Behavior

At the L1 Lagrange point between two bodies, the gravity vectors from both sources nearly cancel, producing a near-zero resultant. This emerges naturally from the vector sum:

```
Planet gravity vector:  →→→→→→→→  (toward planet)
Moon gravity vector:    ←←←←←←←  (toward moon)
Sum at L1:              →        (small residual toward planet)
```

The engine does not need to detect or special-case Lagrange points. They simply exist as regions of low `LocalGravity.magnitude`. Gameplay systems can check for `magnitude < threshold` to trigger zero-g behaviors.

### Blending Continuity Guarantee

The vector sum is inherently continuous: as an entity moves smoothly through space, each source's contribution changes smoothly (the direction rotates continuously and the magnitude follows a smooth inverse-square curve). The sum of smooth functions is smooth. There are no discontinuities anywhere in the gravity field — not at influence boundaries (because the influence cutoff uses a distance threshold, and at that threshold the contribution is already vanishingly small), not at Lagrange points, not at the surface.

### Dominant Source Update System

```rust
fn dominant_source_update_system(
    registry: Res<GravitySourceRegistry>,
    mut entities: Query<(&WorldPos, &mut DominantGravitySource)>,
) {
    for (pos, mut dominant) in entities.iter_mut() {
        if let Some(new_dominant) = find_dominant_source(pos, &registry) {
            *dominant = new_dominant;
        }
    }
}
```

## Outcome

Gravity blending uses physical vector summation of all contributing sources. A `DominantGravitySource` component tracks which single source most strongly affects each entity, along with a dominance fraction for smooth camera and gameplay transitions. Lagrange points emerge naturally from vector cancellation. The gravity field is continuous everywhere with no discontinuities at boundaries. `cargo test -p nebula-gravity` passes all multi-source blending tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Between two planets, gravity vectors from both sources are summed. At the Lagrange point where forces cancel, the player floats weightlessly.

## Crates & Dependencies

- `bevy_ecs = "0.18"` — ECS framework for `Component`, `Res`, `Query`, system scheduling, `Entity` references
- `glam = "0.32"` — `DVec3` for f64 vector accumulation during gravity summation
- `nebula-math` (internal) — `WorldPos` for 128-bit position queries
- `nebula-gravity` (internal) — `GravitySource`, `GravitySourceRegistry`, `compute_gravity_from_source` from stories 01 and 02

## Unit Tests

- **`test_single_source_dominates_near_surface`** — Place one gravity source (planet) at the origin. Place an entity at the surface. Call `compute_gravity` and `find_dominant_source`. Assert `magnitude` approximately equals `surface_gravity`. Assert `dominance` is `1.0` (sole source). Verifies single-source behavior is unaffected by the blending system.

- **`test_two_sources_produce_blended_vector`** — Place planet A at `WorldPos(0, 0, 0)` with `surface_gravity = 9.81, radius = 6_371_000`. Place planet B at `WorldPos(50_000_000, 0, 0)` with `surface_gravity = 3.72, radius = 3_389_500`. Place an entity at `WorldPos(20_000_000, 0, 0)`. Compute gravity. Assert the direction has a positive x-component (pulled more toward the closer/stronger source) and that `magnitude` is greater than either source's contribution alone would suggest a pure single-source scenario. Verifies the vector sum combines both contributions.

- **`test_lagrange_point_has_near_zero_gravity`** — Place two identical gravity sources (`surface_gravity = 9.81, radius = 6_371_000`) at `WorldPos(-25_000_000, 0, 0)` and `WorldPos(25_000_000, 0, 0)`. Place an entity at the exact midpoint `WorldPos(0, 0, 0)`. Compute gravity. Assert `magnitude < 0.01` (near-zero due to cancellation). Verifies that Lagrange-like points emerge naturally from vector summation.

- **`test_dominant_source_changes_between_bodies`** — Same two-source setup. Query `find_dominant_source` at `WorldPos(-10_000_000, 0, 0)` (closer to source A). Assert dominant entity is A. Query at `WorldPos(10_000_000, 0, 0)` (closer to source B). Assert dominant entity is B. Verifies the dominant source transitions as the entity moves between bodies.

- **`test_blending_is_continuous`** — Place two gravity sources. Sample gravity at 100 positions along a straight line between them at even intervals. For each pair of adjacent samples, assert that the change in direction (angle between consecutive direction vectors) is less than 5 degrees and the change in magnitude is less than 10% of the larger value. Verifies no discontinuous jumps in the gravity field.
