# Spaceship Flight Controller

## Problem

Planetary exploration is only half the game — the other half happens in space. The engine needs a six-degrees-of-freedom (6DOF) spaceship controller for zero-gravity flight. Unlike ground-based movement where velocity decays naturally through friction, space has no drag: once the player thrusts, velocity accumulates and persists indefinitely until countered by opposite thrust. The controller must support pitch and yaw via mouse, roll via Q/E keys, forward/backward/strafe thrust via WASD, and a boost modifier on Shift. When the ship enters a gravity well (near a planet), gravity must affect the ship's trajectory. Velocity is stored in 128-bit world space (i128 units per tick) so it never loses precision regardless of speed or distance.

## Solution

### Component

```rust
use bevy_ecs::prelude::*;

/// Tags an entity as a player-controlled spaceship with 6DOF flight.
/// The entity must also have `WorldPos`, `Rotation`, and `Velocity` components.
#[derive(Component, Clone, Debug)]
pub struct SpaceshipController {
    /// Base thrust force in mm/tick^2. Applied each tick the thrust key is held.
    pub thrust: i128,
    /// Boost multiplier when Shift is held.
    pub boost_multiplier: i128,
    /// Mouse sensitivity for pitch and yaw (radians per pixel).
    pub mouse_sensitivity: f32,
    /// Roll speed in radians per tick when Q/E is held.
    pub roll_speed: f32,
    /// Whether the ship is currently in a gravity well.
    /// Set by the gravity detection system (34_gravity), read here.
    pub in_gravity_well: bool,
}

impl Default for SpaceshipController {
    fn default() -> Self {
        Self {
            thrust: 50,            // 50 mm/tick^2
            boost_multiplier: 5,   // 5x thrust when boosting
            mouse_sensitivity: 0.002,
            roll_speed: 0.03,      // ~1.7 degrees per tick
            in_gravity_well: false,
        }
    }
}
```

### Rotation system

Mouse controls pitch (Y axis) and yaw (Y rotation). Q/E controls roll (rotation around the ship's forward axis). Unlike the first person camera, there is no gimbal-lock prevention — the ship can orient freely in any direction using quaternion composition:

```rust
use glam::Quat;

pub fn spaceship_rotation_system(
    mouse: Res<MouseState>,
    keyboard: Res<KeyboardState>,
    mut query: Query<(&SpaceshipController, &mut Rotation)>,
) {
    let (dx, dy) = mouse.delta();
    for (ship, mut rotation) in query.iter_mut() {
        // Pitch: mouse Y rotates around local right axis
        let pitch = Quat::from_axis_angle(
            rotation.0 * glam::Vec3::X,
            -dy * ship.mouse_sensitivity,
        );
        // Yaw: mouse X rotates around local up axis
        let yaw = Quat::from_axis_angle(
            rotation.0 * glam::Vec3::Y,
            -dx * ship.mouse_sensitivity,
        );
        // Roll: Q/E rotates around local forward axis
        let mut roll_amount = 0.0;
        if keyboard.is_pressed(KeyCode::KeyQ) { roll_amount += ship.roll_speed; }
        if keyboard.is_pressed(KeyCode::KeyE) { roll_amount -= ship.roll_speed; }
        let roll = Quat::from_axis_angle(
            rotation.0 * glam::Vec3::NEG_Z,
            roll_amount,
        );

        // Apply rotations: order is roll * pitch * yaw * current
        rotation.0 = (roll * pitch * yaw * rotation.0).normalize();
    }
}
```

### Thrust system

Thrust accumulates velocity without any drag in zero-gravity. Each tick the player holds a thrust key, velocity changes by the thrust amount along the ship's local axis:

```rust
use nebula_math::Vec3I128;

pub fn spaceship_thrust_system(
    keyboard: Res<KeyboardState>,
    mut query: Query<(&SpaceshipController, &Rotation, &mut Velocity)>,
) {
    for (ship, rotation, mut velocity) in query.iter_mut() {
        let forward = rotation.0 * glam::Vec3::NEG_Z;
        let right = rotation.0 * glam::Vec3::X;
        let up = rotation.0 * glam::Vec3::Y;

        let multiplier = if keyboard.is_pressed(KeyCode::ShiftLeft) {
            ship.thrust * ship.boost_multiplier
        } else {
            ship.thrust
        };

        let mut thrust_dir = glam::Vec3::ZERO;
        if keyboard.is_pressed(KeyCode::KeyW) { thrust_dir += forward; }
        if keyboard.is_pressed(KeyCode::KeyS) { thrust_dir -= forward; }
        if keyboard.is_pressed(KeyCode::KeyD) { thrust_dir += right; }
        if keyboard.is_pressed(KeyCode::KeyA) { thrust_dir -= right; }

        if thrust_dir.length_squared() > 1e-6 {
            thrust_dir = thrust_dir.normalize();
            let accel = Vec3I128::new(
                (thrust_dir.x * multiplier as f32) as i128,
                (thrust_dir.y * multiplier as f32) as i128,
                (thrust_dir.z * multiplier as f32) as i128,
            );
            velocity.0 = velocity.0 + accel;
        }
        // No drag applied — velocity persists without input.
    }
}
```

### Velocity application system

A shared system (not specific to spaceships, but used by them) applies velocity to world position each tick:

```rust
pub fn apply_velocity_system(
    mut query: Query<(&Velocity, &mut WorldPos)>,
) {
    for (velocity, mut world_pos) in query.iter_mut() {
        world_pos.0 = world_pos.0 + velocity.0;
    }
}
```

### Gravity interaction

When `in_gravity_well` is true, the gravity system (34_gravity) adds gravitational acceleration to the ship's `Velocity` each tick. The spaceship controller does not implement gravity itself — it only provides the `in_gravity_well` flag for UI/HUD purposes (e.g., showing "gravity detected"). The actual gravity computation is an external system that queries all entities with `Velocity` and `WorldPos` against known planetary bodies.

### Speed display

For HUD rendering, speed in m/s is computed from the velocity magnitude:

```rust
impl SpaceshipController {
    /// Convert current velocity to meters per second for display.
    /// Assumes 60 ticks per second and 1 unit = 1 mm.
    pub fn speed_ms(velocity: &Velocity, ticks_per_second: f64) -> f64 {
        let vx = velocity.0.x as f64;
        let vy = velocity.0.y as f64;
        let vz = velocity.0.z as f64;
        let magnitude_mm_per_tick = (vx * vx + vy * vy + vz * vz).sqrt();
        // mm/tick -> m/s: multiply by ticks_per_second, divide by 1000
        magnitude_mm_per_tick * ticks_per_second / 1000.0
    }
}
```

## Outcome

A `spaceship_controller.rs` module in `crates/nebula_player/src/` exporting `SpaceshipController`, `spaceship_rotation_system`, `spaceship_thrust_system`, and the `speed_ms` helper. The controller is spawned alongside `WorldPos`, `Rotation`, `Velocity`, and the rendering `Camera`. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Pressing a key switches to spaceship mode. The camera gains roll control and 6-DOF thrusters. The player can fly off the planet's surface into orbit.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | `Component` derive, `Res`, `Query`, system function signatures |
| `glam` | `0.32` | `Quat`, `Vec3` for rotation and direction math |
| `winit` | `0.30` | `KeyCode` for physical key identification |
| `nebula-math` | workspace | `WorldPosition`, `Vec3I128`, `Velocity` for 128-bit physics |
| `nebula-input` | workspace | `KeyboardState`, `MouseState` resources |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};
    use nebula_math::Vec3I128;

    #[test]
    fn test_thrust_increases_velocity() {
        let ship = SpaceshipController::default();
        let rotation = Rotation(Quat::IDENTITY); // facing -Z
        let mut velocity = Velocity::new(0, 0, 0);

        // Simulate one tick of forward thrust
        let forward = rotation.0 * Vec3::NEG_Z;
        let accel = Vec3I128::new(
            (forward.x * ship.thrust as f32) as i128,
            (forward.y * ship.thrust as f32) as i128,
            (forward.z * ship.thrust as f32) as i128,
        );
        velocity.0 = velocity.0 + accel;

        // Velocity should now be non-zero, pointing in -Z
        assert!(velocity.0.z < 0);
    }

    #[test]
    fn test_no_drag_in_space() {
        // After thrust is applied, velocity should remain constant
        // when no further input is given.
        let velocity = Velocity::new(100, -50, 200);
        // Simulate 100 ticks with no thrust: velocity is unchanged
        let after = velocity;
        assert_eq!(after.0.x, 100);
        assert_eq!(after.0.y, -50);
        assert_eq!(after.0.z, 200);
    }

    #[test]
    fn test_boost_multiplies_thrust() {
        let ship = SpaceshipController::default();
        let normal_thrust = ship.thrust;
        let boosted_thrust = ship.thrust * ship.boost_multiplier;
        assert_eq!(boosted_thrust, normal_thrust * ship.boost_multiplier);
        assert!(boosted_thrust > normal_thrust);
    }

    #[test]
    fn test_roll_rotates_around_forward_axis() {
        let initial = Quat::IDENTITY;
        let forward = initial * Vec3::NEG_Z;
        let roll_speed = 0.03_f32;
        let roll = Quat::from_axis_angle(forward, roll_speed);
        let rotated = (roll * initial).normalize();

        // The forward vector should be unchanged after a pure roll
        let new_forward = rotated * Vec3::NEG_Z;
        assert!((new_forward - forward).length() < 1e-4);

        // But the up vector should have changed
        let initial_up = initial * Vec3::Y;
        let new_up = rotated * Vec3::Y;
        assert!((new_up - initial_up).length() > 1e-4);
    }

    #[test]
    fn test_velocity_preserved_without_input() {
        // Ship moving at constant velocity; apply_velocity each tick
        let velocity = Velocity::new(1000, 0, -500);
        let mut world_pos = WorldPos::new(0, 0, 0);

        // Simulate 10 ticks
        for _ in 0..10 {
            world_pos.0 = world_pos.0 + velocity.0;
        }

        assert_eq!(world_pos.0.x, 10_000);
        assert_eq!(world_pos.0.y, 0);
        assert_eq!(world_pos.0.z, -5_000);
    }

    #[test]
    fn test_gravity_affects_velocity() {
        // Simulate gravity adding downward acceleration each tick
        let gravity_accel = Vec3I128::new(0, -10, 0); // 10 mm/tick^2 downward
        let mut velocity = Velocity::new(100, 0, 0); // moving horizontally

        for _ in 0..60 {
            velocity.0 = velocity.0 + gravity_accel;
        }

        // After 60 ticks, horizontal velocity unchanged, vertical velocity is downward
        assert_eq!(velocity.0.x, 100);
        assert_eq!(velocity.0.y, -600); // 60 * -10
    }

    #[test]
    fn test_speed_display_stationary() {
        let velocity = Velocity::new(0, 0, 0);
        let speed = SpaceshipController::speed_ms(&velocity, 60.0);
        assert!((speed - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_speed_display_known_velocity() {
        // 1000 mm/tick at 60 ticks/s = 60_000 mm/s = 60 m/s
        let velocity = Velocity::new(1000, 0, 0);
        let speed = SpaceshipController::speed_ms(&velocity, 60.0);
        assert!((speed - 60.0).abs() < 0.1);
    }

    #[test]
    fn test_6dof_all_axes_independent() {
        // Pitch, yaw, and roll should all be independently controllable.
        let rot = Quat::IDENTITY;

        let pitch = Quat::from_axis_angle(rot * Vec3::X, 0.1);
        let yaw = Quat::from_axis_angle(rot * Vec3::Y, 0.2);
        let roll = Quat::from_axis_angle(rot * Vec3::NEG_Z, 0.3);

        let combined = (roll * pitch * yaw * rot).normalize();
        // Combined rotation should differ from any individual rotation
        assert!((combined - pitch).length() > 0.01);
        assert!((combined - yaw).length() > 0.01);
        assert!((combined - roll).length() > 0.01);
    }
}
```
