# Zero-Gravity Physics

## Problem

Nebula Engine spans planetary surfaces and interplanetary space. Between planets — far from any `GravitySource` — entities exist in zero-gravity conditions. Spaceships, debris, cargo containers, and even the player must behave according to Newtonian mechanics: objects at rest stay at rest, objects in motion maintain their velocity indefinitely, and collisions exchange momentum without any gravitational bias. This is fundamentally different from surface physics where gravity constantly pulls everything down. The engine must support both regimes simultaneously (some entities on a planet surface, others floating in space) and handle the unique challenges of zero-g: uncontrolled rotation, momentum conservation, thrust-based movement for ships, and optional angular damping for gameplay feel. Without explicit zero-g support, spaceships would drift unpredictably, collisions in space would feel wrong, and the player experience transitioning from planet to space would be jarring.

## Solution

### Zero-G Detection

An entity is in zero-g when its `LocalGravity` component (from story 07) has a magnitude below a threshold:

```rust
pub const ZERO_G_THRESHOLD: f32 = 0.01; // m/s², below this is "zero gravity"

pub fn is_zero_gravity(gravity: &LocalGravity) -> bool {
    gravity.magnitude < ZERO_G_THRESHOLD
}
```

This is not a binary flag — it emerges naturally from the gravity source computation. An entity gradually transitions into zero-g as it moves away from gravity sources.

### Newtonian Inertia

In standard Rapier simulation, dynamic bodies already obey Newton's laws. With no gravity force applied (magnitude below threshold, so the gravity force system from story 07 applies zero force), a body maintains its linear velocity indefinitely. No special code is needed for this — it is Rapier's default behavior when no external forces act.

However, Rapier has default linear and angular damping that slowly bleeds velocity. For space objects, damping should be zero:

```rust
#[derive(Component)]
pub struct SpaceObject {
    /// If true, this object has zero linear/angular damping (pure Newtonian).
    pub newtonian: bool,
    /// Optional angular damping for gameplay feel (0.0 = none, 1.0 = heavy).
    pub angular_damping_override: Option<f32>,
}

fn configure_space_damping_system(
    physics: ResMut<PhysicsWorld>,
    query: Query<(&RigidBodyHandle, &SpaceObject, &LocalGravity)>,
) {
    for (handle, space_obj, gravity) in query.iter() {
        if let Some(body) = physics.rigid_body_set.get_mut(handle.0) {
            if is_zero_gravity(gravity) && space_obj.newtonian {
                body.set_linear_damping(0.0);
                body.set_angular_damping(
                    space_obj.angular_damping_override.unwrap_or(0.0)
                );
            } else {
                // On a surface, restore default damping.
                body.set_linear_damping(0.5);
                body.set_angular_damping(1.0);
            }
        }
    }
}
```

### Spaceship Thrust

Spaceships move by applying forces (thrust) rather than setting velocity directly. This produces physically correct acceleration and deceleration:

```rust
#[derive(Component)]
pub struct ThrustInput {
    /// Thrust force vector in the ship's local frame (forward, up, right).
    pub linear: glam::Vec3,
    /// Torque vector in the ship's local frame (pitch, yaw, roll).
    pub angular: glam::Vec3,
    /// Maximum linear thrust force in Newtons.
    pub max_thrust: f32,
    /// Maximum torque in N*m.
    pub max_torque: f32,
}

fn apply_thrust_system(
    physics: ResMut<PhysicsWorld>,
    query: Query<(&RigidBodyHandle, &ThrustInput)>,
) {
    for (handle, thrust) in query.iter() {
        if let Some(body) = physics.rigid_body_set.get_mut(handle.0) {
            // Transform thrust from ship-local to world-local frame.
            let rotation = *body.rotation();
            let world_thrust = rotation * vector![
                thrust.linear.x * thrust.max_thrust,
                thrust.linear.y * thrust.max_thrust,
                thrust.linear.z * thrust.max_thrust
            ];
            body.add_force(world_thrust, true);

            let world_torque = rotation * vector![
                thrust.angular.x * thrust.max_torque,
                thrust.angular.y * thrust.max_torque,
                thrust.angular.z * thrust.max_torque
            ];
            body.add_torque(world_torque, true);
        }
    }
}
```

The ship's `ThrustInput` is populated by the player input system (for piloted ships) or by AI controllers (for NPC ships).

### Rotational Physics

In zero-g, rotation is persistent and physically significant. A spaceship that starts spinning continues spinning until counter-torque is applied. The engine exposes angular velocity for gameplay systems:

```rust
pub fn get_angular_velocity(
    physics: &PhysicsWorld,
    handle: &RigidBodyHandle,
) -> Option<glam::Vec3> {
    physics.rigid_body_set.get(handle.0).map(|body| {
        let av = body.angvel();
        glam::Vec3::new(av.x, av.y, av.z)
    })
}
```

### Optional Angular Damping for Gameplay

Pure Newtonian rotation can feel disorienting for players. The `angular_damping_override` field allows ships to have a small amount of damping that gradually slows rotation when no torque is applied. This is unrealistic but improves gameplay feel. The default for player-piloted ships is `Some(0.3)` — enough to stabilize eventually, but not so much that rotation feels sluggish. AI ships and debris use `None` (pure Newtonian) or `Some(0.0)`.

### Collision Momentum Conservation

Rapier's collision solver naturally conserves momentum. In zero-g, this means:
- Two equal-mass objects colliding head-on exchange velocities.
- A fast-moving object hitting a stationary one transfers momentum proportionally to mass ratio.
- Elastic vs. inelastic collisions are controlled by the `restitution` coefficient on colliders.

No special code is needed — Rapier handles this correctly when gravity forces are zero. However, the engine sets a default `restitution` of 0.3 for space objects (slightly bouncy) versus 0.0 for terrain (no bounce).

### Micro-Gravity Zones

Some locations (e.g., asteroid interiors, between two close planets) have very low but nonzero gravity. The engine does not clamp to pure zero — it lets the gravity system provide whatever tiny value exists. Objects in these zones drift slowly in a direction, creating emergent gameplay scenarios.

## Outcome

Entities in space far from gravity sources experience true Newtonian physics: constant velocity, momentum-conserving collisions, thrust-based maneuvering, and persistent rotation. Spaceships feel weighty and physically plausible. Angular damping can be tuned per-object for gameplay balance. `cargo test -p nebula-physics` passes all zero-gravity behavior tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Flying far from the planet, gravity diminishes to zero. The player floats freely. Pushing off a surface sends you drifting with no deceleration — true zero-g.

## Crates & Dependencies

- `rapier3d = "0.32"` — Dynamic rigid bodies with configurable damping, force/torque application, collision solver with momentum conservation
- `bevy_ecs = "0.18"` — ECS framework for components (`SpaceObject`, `ThrustInput`), systems, and queries
- `glam = "0.32"` — Vector math for thrust calculations and angular velocity queries
- `nebula-physics` (internal, self) — `PhysicsWorld` resource, `LocalGravity` component, `ZERO_G_THRESHOLD`

## Unit Tests

- **`test_object_in_space_maintains_velocity`** — Create a dynamic body in an empty world (no gravity sources). Set its linear velocity to `(10, 0, 0)`. Step physics 600 times (10 seconds). Assert the body's position is approximately `(100, 0, 0)` — velocity maintained without decay. Linear damping must be 0.

- **`test_no_acceleration_without_thrust`** — Create a body at rest in zero-g. Apply no forces. Step physics 120 times. Assert the body's position has not changed (remains at the origin within f32 epsilon). Newton's first law: a body at rest stays at rest.

- **`test_collision_in_zero_g_conserves_momentum`** — Create two dynamic bodies of equal mass, one moving at `(10, 0, 0)` and one stationary. Position them on a collision course. Step until they collide and separate. Compute total momentum (mass * velocity) before and after. Assert total momentum is conserved within 1% tolerance.

- **`test_torque_causes_rotation`** — Create a dynamic body in zero-g with zero angular damping. Apply a torque of `(0, 100, 0)` for one tick. Assert angular velocity y-component is greater than 0 after stepping. Step 60 more ticks without torque. Assert angular velocity is still approximately the same (no damping).

- **`test_angular_damping_slows_rotation`** — Create a body with `angular_damping_override = Some(2.0)`. Set initial angular velocity to `(0, 10, 0)`. Step 120 ticks. Assert angular velocity magnitude is significantly less than 10 (damped). Verify it approaches zero over time.

- **`test_thrust_produces_acceleration`** — Create a stationary spaceship body. Apply a `ThrustInput` with `linear = (0, 0, 1)` and `max_thrust = 1000.0` for 60 ticks. Assert the body's velocity z-component is greater than 0 and position has moved in the +z direction. Remove thrust, step 60 more ticks. Assert velocity is maintained (no damping).
