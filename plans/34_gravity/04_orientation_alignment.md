# Orientation Alignment

## Problem

On a cubesphere planet, "up" is not a constant direction — it varies continuously across the surface, pointing radially away from the planet center. An entity standing on the north pole has a different "up" than one on the equator, and as an entity walks around the planet, its orientation must rotate to stay upright relative to the local gravity direction. Snapping orientation instantly would cause visual jarring and physics instability. The rotation must be smooth (interpolated over time) and configurable in speed. Additionally, spaceships and other vehicles must be able to override this alignment entirely to fly in any orientation. Without orientation alignment, characters would appear to lean at extreme angles or fall over when walking across the cubesphere surface.

## Solution

### GravityAlignment Component

A component that controls how an entity aligns to the local gravity direction:

```rust
use bevy_ecs::component::Component;

/// Controls how an entity orients itself relative to local gravity.
#[derive(Component, Debug, Clone)]
pub struct GravityAlignment {
    /// The current "up" direction of the entity (normalized, world space).
    /// This is the direction the entity considers "away from ground."
    pub current_up: glam::Vec3,

    /// Rotation speed in radians per second.
    /// Higher values = snappier alignment. Typical: 5.0-10.0 for characters.
    pub alignment_speed: f32,

    /// If true, the entity ignores gravity-based alignment entirely.
    /// Used for ships in free flight, zero-g, or cinematic sequences.
    pub override_active: bool,

    /// When override is active, this is the manually-set "up" direction.
    pub override_up: glam::Vec3,
}

impl Default for GravityAlignment {
    fn default() -> Self {
        Self {
            current_up: glam::Vec3::Y,
            alignment_speed: 8.0,
            override_active: false,
            override_up: glam::Vec3::Y,
        }
    }
}
```

### Alignment Computation

The alignment system takes the entity's `LocalGravity` direction (from story 02), computes the target "up" as the negation of gravity direction (gravity points down, up is the opposite), and smoothly rotates `current_up` toward the target:

```rust
use glam::{Quat, Vec3};

/// Compute the incremental rotation to align `current_up` toward `target_up`.
///
/// Uses spherical linear interpolation (slerp) bounded by maximum rotation
/// per tick to ensure smooth, speed-limited alignment.
pub fn compute_alignment_rotation(
    current_up: Vec3,
    target_up: Vec3,
    alignment_speed: f32,
    dt: f32,
) -> Quat {
    // If already aligned (or nearly so), return identity.
    let dot = current_up.dot(target_up).clamp(-1.0, 1.0);
    if dot > 0.99999 {
        return Quat::IDENTITY;
    }

    // Compute the rotation from current_up to target_up.
    let full_rotation = Quat::from_rotation_arc(current_up, target_up);

    // Limit the rotation by the alignment speed and delta time.
    // The slerp parameter is clamped to [0, 1] so we never overshoot.
    let max_angle = alignment_speed * dt;
    let full_angle = full_rotation.to_axis_angle().1;
    let t = (max_angle / full_angle).min(1.0);

    Quat::IDENTITY.slerp(full_rotation, t)
}
```

### Alignment System

Runs after the gravity field update in `FixedUpdate`:

```rust
fn gravity_alignment_system(
    time: Res<FixedTime>,
    mut entities: Query<(
        &LocalGravity,
        &mut GravityAlignment,
        &mut Transform,
    )>,
) {
    let dt = time.period.as_secs_f32();

    for (gravity, mut alignment, mut transform) in entities.iter_mut() {
        if alignment.override_active {
            alignment.current_up = alignment.override_up;
            continue;
        }

        // Target up is opposite to gravity direction.
        // If gravity magnitude is zero (deep space), keep current orientation.
        if gravity.magnitude < 1e-6 {
            continue;
        }

        let target_up = -gravity.direction;
        let rotation = compute_alignment_rotation(
            alignment.current_up,
            target_up,
            alignment.alignment_speed,
            dt,
        );

        // Apply the incremental rotation to the entity's transform.
        transform.rotation = rotation * transform.rotation;

        // Update the tracked up direction.
        alignment.current_up = (rotation * alignment.current_up).normalize();
    }
}
```

### Pole Handling

At the geographic poles of a cubesphere planet, the gravity direction is well-defined (pointing straight down toward the center), so alignment works correctly. The potential issue is that the entity's forward/right directions become ambiguous at poles — this is the "gimbal lock" problem. The solution is that `GravityAlignment` only constrains the up axis; the entity's yaw (rotation around the up axis) remains free and is controlled by the input/movement system. `Quat::from_rotation_arc` handles the pole case correctly because it computes the shortest-arc rotation, which is always well-defined when the source and target vectors are not exactly anti-parallel.

### Ship Override

When a ship enters free flight, it sets `override_active = true` and controls its own orientation through thrust vectoring. The alignment system skips the entity entirely. When the ship enters a gravity well and the pilot engages landing mode, `override_active` is set to `false` and the ship smoothly aligns to the surface over several seconds (using a low `alignment_speed` like 2.0 for large vessels).

## Outcome

Every entity with a `GravityAlignment` component smoothly orients its "up" direction to oppose the local gravity vector. The rotation is rate-limited by `alignment_speed` and uses slerp for smooth interpolation. Ships and special entities can override alignment for free-flight orientation. The system handles cubesphere poles correctly. `cargo test -p nebula-gravity` passes all orientation alignment tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Walking on a planet, the player's "up" vector gradually aligns to the surface normal. Walking from pole to equator smoothly rotates the world so ground is always down.

## Crates & Dependencies

- `bevy_ecs = "0.18"` — ECS framework for `Component`, `Query`, `Res`, system scheduling, `Transform` access
- `glam = "0.32"` — `Quat` for rotation math, `Vec3` for direction vectors, `Quat::from_rotation_arc` and `slerp` for smooth interpolation
- `nebula-gravity` (internal) — `LocalGravity` component from story 02 for current gravity direction

## Unit Tests

- **`test_entity_aligns_to_gravity_direction`** — Create a `GravityAlignment` with `current_up = Vec3::Y`. Set `LocalGravity { direction: Vec3::new(0.0, -1.0, 0.0), magnitude: 9.81 }` (standard downward gravity). Run the alignment system for one tick. Assert `current_up` is still approximately `Vec3::Y` (already aligned — gravity down means up is Y). Change gravity direction to `Vec3::new(-1.0, 0.0, 0.0)` (gravity points in -X). Run several ticks. Assert `current_up` has rotated toward `Vec3::X` (away from gravity).

- **`test_alignment_is_smooth_lerped`** — Set `current_up = Vec3::Y` and target up = `Vec3::X` (90 degrees apart). Set `alignment_speed = 2.0` and `dt = 1.0/60.0`. Call `compute_alignment_rotation`. Assert the resulting rotation angle is approximately `2.0 / 60.0` radians (~1.9 degrees), not the full 90 degrees. Verifies the rotation is rate-limited and does not snap instantly.

- **`test_alignment_changes_as_entity_moves_on_surface`** — Simulate an entity moving around a planet: at position `(R, 0, 0)`, gravity direction is `(-1, 0, 0)`, so up should be `(1, 0, 0)`. Move to `(0, R, 0)`, gravity direction is `(0, -1, 0)`, up should be `(0, 1, 0)`. Run alignment for enough ticks for convergence at each position. Assert `current_up` has rotated from `(1, 0, 0)` toward `(0, 1, 0)`.

- **`test_ship_can_override_alignment`** — Set `override_active = true` and `override_up = Vec3::new(0.0, 0.0, 1.0)`. Run the alignment system. Assert `current_up` equals `Vec3::new(0.0, 0.0, 1.0)` regardless of the `LocalGravity` direction. Verifies the override bypasses gravity-based alignment.

- **`test_alignment_at_poles_is_correct`** — Place an entity at the "north pole" of a planet: `WorldPos(0, R, 0)` where gravity direction is `(0, -1, 0)`. Compute alignment target up: `(0, 1, 0)`. Set `current_up = Vec3::new(0.001, 1.0, 0.0).normalize()` (slightly off-axis). Run alignment. Assert `current_up` converges toward `(0, 1, 0)` without NaN or degenerate quaternion values. Verifies numerical stability at poles.
