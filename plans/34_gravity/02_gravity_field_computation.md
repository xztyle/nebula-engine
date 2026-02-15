# Gravity Field Computation

## Problem

For every physics-active entity in the simulation, the engine must determine the gravity vector — both its direction and its magnitude — at that entity's world position. On the surface of a cubesphere planet, gravity points toward the planet center. At altitude, gravity weakens according to the inverse-square law. Far from all gravity sources, gravity is zero. The computation must handle 128-bit world coordinates for direction (to avoid precision loss at planetary distances), use f64 intermediate math for magnitude (to avoid f32 catastrophic cancellation), and produce a final `Vec3` direction + `f32` magnitude result suitable for Rapier force application. Without correct field computation, entities will not fall correctly, will not orient properly, and the entire gravity system collapses.

## Solution

### GravityResult Type

The output of a gravity field computation for a single position:

```rust
/// The computed gravity at a specific world position.
#[derive(Debug, Clone, Copy)]
pub struct GravityResult {
    /// Normalized direction of gravity (points "down" — toward the attractor).
    pub direction: glam::Vec3,
    /// Magnitude of gravitational acceleration in m/s².
    pub magnitude: f32,
}
```

### Core Computation Function

For a given world position and a list of gravity sources, compute the resulting gravity vector:

```rust
use glam::DVec3;

/// Compute the gravity vector at `entity_pos` from a single `GravitySource`.
///
/// Uses the formula: g(r) = surface_gravity * (radius / distance)²
/// Direction: from entity toward source center.
///
/// Returns `None` if the entity is outside the source's influence radius
/// or at the source's exact center (degenerate case).
pub fn compute_gravity_from_source(
    entity_pos: &WorldPos,
    source_pos: &WorldPos,
    source: &GravitySource,
) -> Option<(DVec3, f64)> {
    // Compute displacement in i128, then convert to f64 for magnitude.
    let dx = (source_pos.x - entity_pos.x) as f64;
    let dy = (source_pos.y - entity_pos.y) as f64;
    let dz = (source_pos.z - entity_pos.z) as f64;

    let distance_sq = dx * dx + dy * dy + dz * dz;
    let distance = distance_sq.sqrt();

    // Guard: outside influence range or degenerate (at center).
    if distance > source.influence_radius as f64 || distance < 1.0 {
        return None;
    }

    let direction = DVec3::new(dx / distance, dy / distance, dz / distance);

    // g(r) = surface_gravity * (radius / distance)²
    let radius_f64 = source.radius as f64;
    let ratio = radius_f64 / distance;
    let magnitude = source.surface_gravity as f64 * ratio * ratio;

    Some((direction, magnitude))
}

/// Compute the combined gravity vector at `entity_pos` from all sources.
///
/// Sums the acceleration vectors from each contributing source.
/// Returns a normalized direction and total magnitude.
pub fn compute_gravity(
    entity_pos: &WorldPos,
    sources: &[(WorldPos, &GravitySource)],
) -> GravityResult {
    let mut total_accel = DVec3::ZERO;

    for (source_pos, source) in sources {
        if let Some((direction, magnitude)) = compute_gravity_from_source(
            entity_pos, source_pos, source,
        ) {
            total_accel += direction * magnitude;
        }
    }

    let total_mag = total_accel.length();
    if total_mag < 1e-9 {
        return GravityResult {
            direction: glam::Vec3::NEG_Y, // Default "down" when no gravity
            magnitude: 0.0,
        };
    }

    GravityResult {
        direction: glam::Vec3::new(
            (total_accel.x / total_mag) as f32,
            (total_accel.y / total_mag) as f32,
            (total_accel.z / total_mag) as f32,
        ),
        magnitude: total_mag as f32,
    }
}
```

### Why i128 for Direction, f64 for Magnitude

Direction computation begins in i128: the displacement `source_pos - entity_pos` is computed in the engine's native 128-bit coordinate space to avoid overflow or precision loss when two positions are far apart (e.g., an entity near one planet computing gravity from a distant moon). The i128 differences are then cast to f64 for normalization and magnitude calculations. The final direction is stored as `Vec3` (f32) because the physics engine and renderer consume f32 vectors, and direction does not require double precision.

Magnitude is computed in f64 because the inverse-square law involves squaring potentially large distances. An f64 has 52 bits of mantissa, which can represent distances up to ~4.5e15 without precision loss in the ratio computation. The final magnitude is cast to f32 for Rapier consumption.

### At-Surface Shortcut

At exactly the surface radius (`distance == radius`), the formula simplifies to `surface_gravity * 1.0 = surface_gravity`. This is the pre-computation advantage: no gravitational constant `G` or mass `M` appears in the per-tick computation. The `GravitySource` component already stores the result of `G*M/R²` as `surface_gravity`.

### Gravity Update System

A system runs in `FixedUpdate` to compute and cache gravity for every physics entity:

```rust
#[derive(Component, Debug, Clone, Copy)]
pub struct LocalGravity {
    pub direction: glam::Vec3,
    pub magnitude: f32,
}

fn gravity_field_update_system(
    registry: Res<GravitySourceRegistry>,
    mut entities: Query<(&WorldPos, &mut LocalGravity)>,
) {
    for (entity_pos, mut gravity) in entities.iter_mut() {
        let affecting = registry.sources_affecting(entity_pos);
        let source_list: Vec<_> = affecting
            .iter()
            .map(|entry| (entry.position, &entry.source))
            .collect();
        let result = compute_gravity(entity_pos, &source_list);
        gravity.direction = result.direction;
        gravity.magnitude = result.magnitude;
    }
}
```

## Outcome

The `nebula-gravity` crate exports `compute_gravity`, `compute_gravity_from_source`, `GravityResult`, and the `LocalGravity` component. The `gravity_field_update_system` runs each `FixedUpdate` tick, populating every physics entity's `LocalGravity` with the current gravity direction and magnitude. Downstream systems (force application, orientation alignment, character controller) read `LocalGravity` without recomputing. `cargo test -p nebula-gravity` passes all field computation tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Gravity force is computed per-entity based on distance from each source. The debug overlay shows gravity vectors as arrows at sample points in space.

## Crates & Dependencies

- `rapier3d = "0.32"` — Consumes the gravity result as per-body forces; world gravity remains zero
- `bevy_ecs = "0.18"` — ECS framework for `Component`, `Res`, `Query`, system scheduling in `FixedUpdate`
- `glam = "0.32"` — `DVec3` for f64 intermediate computation, `Vec3` for final direction output
- `nebula-math` (internal) — `WorldPos` with i128 fields for displacement computation
- `nebula-gravity` (internal) — `GravitySource`, `GravitySourceRegistry` from story 01

## Unit Tests

- **`test_gravity_at_surface_equals_surface_gravity`** — Create a `GravitySource` with `surface_gravity = 9.81` and `radius = 6_371_000`. Place the source at `WorldPos(0, 0, 0)`. Place an entity at `WorldPos(0, 6_371_000, 0)` (exactly at surface radius). Compute gravity. Assert `magnitude` is approximately `9.81` within `0.01` tolerance. Verifies the formula yields `surface_gravity` at exactly the surface distance.

- **`test_gravity_at_double_radius_is_one_quarter`** — Same source as above. Place the entity at `WorldPos(0, 12_742_000, 0)` (twice the radius). Compute gravity. Assert `magnitude` is approximately `9.81 / 4.0 = 2.4525` within `0.01`. Verifies the inverse-square falloff: at 2x distance, gravity is 1/4.

- **`test_direction_points_toward_center`** — Place a gravity source at `WorldPos(0, 0, 0)`. Place an entity at `WorldPos(6_371_000, 0, 0)`. Compute gravity. Assert `direction` is approximately `(-1.0, 0.0, 0.0)` — pointing from the entity toward the source center. Repeat for `WorldPos(0, 0, -6_371_000)` and assert direction is `(0.0, 0.0, 1.0)`.

- **`test_zero_gravity_far_away`** — Create a source with `influence_radius = 50_000_000`. Place an entity at `WorldPos(0, 100_000_000, 0)` (beyond influence). Compute gravity. Assert `magnitude == 0.0`. Verifies the influence radius cutoff produces exactly zero gravity.

- **`test_gravity_is_a_vector`** — Place a source at `WorldPos(1_000_000, 1_000_000, 0)`. Place an entity at `WorldPos(0, 0, 0)`. Compute gravity. Assert `direction` is approximately `(0.7071, 0.7071, 0.0)` (normalized diagonal). Assert `magnitude > 0.0`. Verifies the result is a proper vector with both direction and magnitude, not just a scalar.
