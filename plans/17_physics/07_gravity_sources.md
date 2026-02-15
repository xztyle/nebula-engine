# Gravity Sources

## Problem

In a game engine with multiple planets, moons, and space stations, gravity is not a simple downward vector. Each celestial body exerts gravity toward its center, and an entity's "down" direction depends on which body it is nearest to. A player standing on the north pole of a planet experiences gravity in the -y direction (in planet-local space), while a player on the equator 90 degrees around experiences gravity in the -x direction. In space between planets, gravity from multiple sources blends together. The engine must compute per-entity gravity direction and magnitude every tick, drive the physics simulation with it, and smoothly transition between gravitational fields. Without this, the engine can only support flat-world games — the cubesphere planetary geometry demands spherical gravity.

## Solution

### GravitySource Component

Any entity that exerts gravity receives a `GravitySource` component:

```rust
#[derive(Component)]
pub struct GravitySource {
    /// Mass of the body, in engine mass units.
    /// Determines gravitational pull strength.
    pub mass: f64,
    /// Surface gravity in m/s² — used as an override for gameplay tuning.
    /// If set, replaces the computed inverse-square value at the surface radius.
    pub surface_gravity: f32,
    /// Radius of the body's surface in meters.
    /// Used to define where "surface gravity" applies.
    pub surface_radius: f64,
    /// Maximum influence radius in meters.
    /// Beyond this, the source contributes zero gravity.
    pub influence_radius: f64,
    /// If true, gravity within the atmosphere is approximately constant
    /// (no inverse-square falloff near the surface). Simplifies surface gameplay.
    pub constant_near_surface: bool,
    /// Atmosphere height in meters. Within surface_radius + atmosphere_height,
    /// gravity magnitude equals surface_gravity if constant_near_surface is true.
    pub atmosphere_height: f64,
}
```

Planets, moons, asteroids, and space stations can all be gravity sources. A typical Earth-like planet would have `surface_gravity = 9.81`, `surface_radius = 6_371_000.0` (in engine units), and `influence_radius` matching its sphere of influence.

### Gravity Computation

For each physics-active entity, compute the combined gravity vector from all sources:

```rust
pub struct GravityResult {
    /// Direction of gravity (normalized, points "down").
    pub direction: glam::Vec3,
    /// Magnitude in m/s².
    pub magnitude: f32,
}

pub fn compute_gravity(
    entity_pos: &WorldPos,
    sources: &[(WorldPos, &GravitySource)],
) -> GravityResult {
    let mut total_accel = glam::DVec3::ZERO;

    for (source_pos, source) in sources {
        let delta_x = (source_pos.x - entity_pos.x) as f64;
        let delta_y = (source_pos.y - entity_pos.y) as f64;
        let delta_z = (source_pos.z - entity_pos.z) as f64;
        let distance_sq = delta_x * delta_x + delta_y * delta_y + delta_z * delta_z;
        let distance = distance_sq.sqrt();

        if distance > source.influence_radius || distance < 1.0 {
            continue; // Outside influence range or degenerate (at center).
        }

        let direction = glam::DVec3::new(
            delta_x / distance,
            delta_y / distance,
            delta_z / distance,
        );

        let magnitude = if source.constant_near_surface
            && distance <= source.surface_radius as f64 + source.atmosphere_height
        {
            // Constant gravity within atmosphere.
            source.surface_gravity as f64
        } else {
            // Inverse-square falloff from the surface value.
            // g(r) = g_surface * (r_surface / r)^2
            let ratio = source.surface_radius / distance;
            source.surface_gravity as f64 * ratio * ratio
        };

        total_accel += direction * magnitude;
    }

    let total_mag = total_accel.length();
    if total_mag < 1e-6 {
        return GravityResult {
            direction: glam::Vec3::NEG_Y, // Default "down" if no gravity
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

### Gravity Update System

A system runs early in `FixedUpdate` to compute and cache gravity for each physics entity:

```rust
#[derive(Component)]
pub struct LocalGravity {
    pub direction: glam::Vec3,
    pub magnitude: f32,
}

fn gravity_update_system(
    sources: Query<(&WorldPos, &GravitySource)>,
    mut entities: Query<(&WorldPos, &mut LocalGravity), With<RigidBodyHandle>>,
) {
    let source_list: Vec<_> = sources.iter().collect();

    for (entity_pos, mut gravity) in entities.iter_mut() {
        let result = compute_gravity(entity_pos, &source_list);
        gravity.direction = result.direction;
        gravity.magnitude = result.magnitude;
    }
}
```

This cached `LocalGravity` is used by the character controller (story 05), dynamic body force application, and any system that needs to know "which way is down" for a given entity.

### Per-Body Force Application

For dynamic rigid bodies (not kinematic characters), gravity is applied as a force each tick rather than using Rapier's global gravity vector:

```rust
fn apply_gravity_forces_system(
    physics: ResMut<PhysicsWorld>,
    query: Query<(&RigidBodyHandle, &LocalGravity)>,
) {
    for (handle, gravity) in query.iter() {
        if let Some(body) = physics.rigid_body_set.get_mut(handle.0) {
            if body.is_dynamic() {
                let mass = body.mass();
                let force = gravity.direction * gravity.magnitude * mass;
                body.add_force(vector![force.x, force.y, force.z], true);
            }
        }
    }
}
```

Rapier's world gravity is set to `(0, 0, 0)` — all gravity comes from per-body forces. This allows different entities to experience different gravity directions simultaneously (one on the north pole, one on the equator).

### Smooth Transitions

When an entity moves between the gravitational influence of two bodies (e.g., flying from a planet to its moon), the `compute_gravity` function naturally blends the gravity vectors because it sums contributions from all sources. The transition is continuous and smooth — there is no hard boundary where gravity direction flips.

For the Lagrange point (equal pull from two bodies), the gravity vectors cancel out, producing near-zero gravity. This is physically correct and creates interesting gameplay possibilities.

## Outcome

Every physics-active entity has a `LocalGravity` component updated each tick based on nearby `GravitySource` entities. Gravity points toward the nearest planet's center on the surface, blends smoothly in space, and reaches zero far from any source. The character controller and dynamic bodies use per-entity gravity rather than a global direction. `cargo test -p nebula-physics` passes all gravity computation tests.

## Demo Integration

**Demo crate:** `nebula-demo`

The planet is registered as a gravity source. Gravity pulls toward the planet center so the player sticks to the surface at any position on the sphere.

## Crates & Dependencies

- `rapier3d = "0.32"` — Per-body force application for gravity; world gravity set to zero
- `bevy_ecs = "0.18"` — ECS framework for components (`GravitySource`, `LocalGravity`), systems, and queries
- `glam = "0.32"` — Vector math for gravity direction and magnitude calculations; `DVec3` for intermediate precision
- `nebula-math` (internal) — `WorldPos` (i128) for distance calculations between entities and gravity sources
- `nebula-coords` (internal) — Coordinate utilities for world-space distance computation

## Unit Tests

- **`test_gravity_points_toward_planet_center`** — Place a `GravitySource` at `WorldPos(0, 0, 0)`. Place an entity at `WorldPos(0, 6_371_000, 0)` (directly above the planet center). Compute gravity. Assert `direction` is approximately `(0, -1, 0)` — gravity points downward toward the center. Move the entity to `WorldPos(6_371_000, 0, 0)`. Assert direction is approximately `(-1, 0, 0)`.

- **`test_surface_gravity_magnitude`** — Create a source with `surface_gravity = 9.81` and `surface_radius = 6_371_000`. Place an entity at exactly `surface_radius` distance from the center. Compute gravity. Assert `magnitude` is approximately `9.81` within 0.01 tolerance.

- **`test_zero_gravity_far_from_sources`** — Create a gravity source with `influence_radius = 1_000_000`. Place an entity at 2,000,000m from the source (beyond influence). Compute gravity. Assert `magnitude == 0.0`. Verifies the influence radius cutoff.

- **`test_two_planets_create_blended_gravity`** — Place two gravity sources 10,000,000m apart, each with `surface_gravity = 9.81`. Place an entity at the midpoint. Compute gravity. Assert `magnitude` is approximately zero (the two vectors cancel). Move the entity 1,000,000m toward one source. Assert gravity points toward the closer source with a nonzero magnitude.

- **`test_gravity_direction_changes_as_entity_orbits`** — Place a gravity source at the origin. Move an entity in a circle at fixed radius: `(R, 0, 0)`, `(0, R, 0)`, `(0, 0, R)`, `(-R, 0, 0)`. At each position, compute gravity and assert the direction points toward `(0, 0, 0)` — i.e., the direction vector is the negation of the entity's normalized position vector.

- **`test_constant_near_surface_flag`** — Create a source with `constant_near_surface = true`, `atmosphere_height = 100_000`. Place entity at `surface_radius + 50_000` (within atmosphere). Assert magnitude equals `surface_gravity` exactly. Move to `surface_radius + 200_000` (above atmosphere). Assert magnitude is less than `surface_gravity` (inverse-square falloff applies).
