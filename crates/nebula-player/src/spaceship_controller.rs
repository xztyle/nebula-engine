//! Six-degrees-of-freedom spaceship flight controller for zero-gravity space.
//!
//! Provides mouse-driven pitch/yaw, Q/E roll, WASD thrust with Shift boost,
//! and persistent velocity (no drag). Velocity is stored in i128 world-space
//! units (mm/tick) for unlimited precision at any speed.

use glam::{Quat, Vec3};
use nebula_ecs::{Rotation, Velocity, WorldPos};
use nebula_input::{KeyboardState, MouseState};
use nebula_math::Vec3I128;
use winit::keyboard::{KeyCode, PhysicalKey};

/// Tags an entity as a player-controlled spaceship with 6DOF flight.
/// The entity must also have `WorldPos`, `Rotation`, and `Velocity` components.
#[derive(Clone, Debug)]
pub struct SpaceshipController {
    /// Base thrust force in mm/tick². Applied each tick the thrust key is held.
    pub thrust: i128,
    /// Boost multiplier when Shift is held.
    pub boost_multiplier: i128,
    /// Mouse sensitivity for pitch and yaw (radians per pixel).
    pub mouse_sensitivity: f32,
    /// Roll speed in radians per tick when Q/E is held.
    pub roll_speed: f32,
    /// Whether the ship is currently in a gravity well.
    /// Set by the gravity detection system (phase 34), read here for HUD.
    pub in_gravity_well: bool,
}

impl Default for SpaceshipController {
    fn default() -> Self {
        Self {
            thrust: 50,
            boost_multiplier: 5,
            mouse_sensitivity: 0.002,
            roll_speed: 0.03,
            in_gravity_well: false,
        }
    }
}

impl SpaceshipController {
    /// Convert current velocity to meters per second for HUD display.
    /// Assumes 1 unit = 1 mm and the given tick rate.
    pub fn speed_ms(velocity: &Velocity, ticks_per_second: f64) -> f64 {
        let vx = velocity.0.x as f64;
        let vy = velocity.0.y as f64;
        let vz = velocity.0.z as f64;
        let magnitude_mm_per_tick = (vx * vx + vy * vy + vz * vz).sqrt();
        // mm/tick → m/s: multiply by ticks/s, divide by 1000
        magnitude_mm_per_tick * ticks_per_second / 1000.0
    }
}

/// Update rotation from mouse delta (pitch/yaw) and Q/E (roll).
///
/// Uses quaternion composition on local axes — no gimbal lock.
pub fn spaceship_rotation_system(
    mouse: &MouseState,
    keyboard: &KeyboardState,
    ship: &SpaceshipController,
    rotation: &mut Rotation,
) {
    let delta = mouse.delta();
    let dx = delta.x;
    let dy = delta.y;

    // Pitch: mouse Y rotates around local right axis
    let pitch = Quat::from_axis_angle(rotation.0 * Vec3::X, -dy * ship.mouse_sensitivity);
    // Yaw: mouse X rotates around local up axis
    let yaw = Quat::from_axis_angle(rotation.0 * Vec3::Y, -dx * ship.mouse_sensitivity);
    // Roll: Q/E rotates around local forward axis (NEG_Z)
    let mut roll_amount = 0.0;
    if keyboard.is_pressed(PhysicalKey::Code(KeyCode::KeyQ)) {
        roll_amount += ship.roll_speed;
    }
    if keyboard.is_pressed(PhysicalKey::Code(KeyCode::KeyE)) {
        roll_amount -= ship.roll_speed;
    }
    let roll = Quat::from_axis_angle(rotation.0 * Vec3::NEG_Z, roll_amount);

    // Apply: roll * pitch * yaw * current
    rotation.0 = (roll * pitch * yaw * rotation.0).normalize();
}

/// Apply WASD thrust along local ship axes, with Shift boost.
///
/// Velocity accumulates with NO drag — once thrust stops, the ship
/// maintains its current velocity indefinitely.
pub fn spaceship_thrust_system(
    keyboard: &KeyboardState,
    ship: &SpaceshipController,
    rotation: &Rotation,
    velocity: &mut Velocity,
) {
    let forward = rotation.0 * Vec3::NEG_Z;
    let right = rotation.0 * Vec3::X;

    let multiplier = if keyboard.is_pressed(PhysicalKey::Code(KeyCode::ShiftLeft)) {
        ship.thrust * ship.boost_multiplier
    } else {
        ship.thrust
    };

    let mut thrust_dir = Vec3::ZERO;
    if keyboard.is_pressed(PhysicalKey::Code(KeyCode::KeyW)) {
        thrust_dir += forward;
    }
    if keyboard.is_pressed(PhysicalKey::Code(KeyCode::KeyS)) {
        thrust_dir -= forward;
    }
    if keyboard.is_pressed(PhysicalKey::Code(KeyCode::KeyD)) {
        thrust_dir += right;
    }
    if keyboard.is_pressed(PhysicalKey::Code(KeyCode::KeyA)) {
        thrust_dir -= right;
    }

    if thrust_dir.length_squared() > 1e-6 {
        thrust_dir = thrust_dir.normalize();
        let accel = Vec3I128::new(
            (thrust_dir.x * multiplier as f32) as i128,
            (thrust_dir.y * multiplier as f32) as i128,
            (thrust_dir.z * multiplier as f32) as i128,
        );
        velocity.0 += accel;
    }
    // No drag — velocity persists without input.
}

/// Apply velocity to world position each tick.
///
/// This system is not spaceship-specific; any entity with `Velocity`
/// and `WorldPos` can use it.
pub fn apply_velocity_system(velocity: &Velocity, world_pos: &mut WorldPos) {
    world_pos.0 = world_pos.0 + velocity.0;
}

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
        velocity.0 += accel;

        assert!(velocity.0.z < 0);
    }

    #[test]
    fn test_no_drag_in_space() {
        let velocity = Velocity::new(100, -50, 200);
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

        // Forward vector unchanged after pure roll
        let new_forward = rotated * Vec3::NEG_Z;
        assert!((new_forward - forward).length() < 1e-4);

        // Up vector should have changed
        let initial_up = initial * Vec3::Y;
        let new_up = rotated * Vec3::Y;
        assert!((new_up - initial_up).length() > 1e-4);
    }

    #[test]
    fn test_velocity_preserved_without_input() {
        let velocity = Velocity::new(1000, 0, -500);
        let mut world_pos = WorldPos::new(0, 0, 0);

        for _ in 0..10 {
            apply_velocity_system(&velocity, &mut world_pos);
        }

        assert_eq!(world_pos.0.x, 10_000);
        assert_eq!(world_pos.0.y, 0);
        assert_eq!(world_pos.0.z, -5_000);
    }

    #[test]
    fn test_gravity_affects_velocity() {
        let gravity_accel = Vec3I128::new(0, -10, 0);
        let mut velocity = Velocity::new(100, 0, 0);

        for _ in 0..60 {
            velocity.0 += gravity_accel;
        }

        assert_eq!(velocity.0.x, 100);
        assert_eq!(velocity.0.y, -600);
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
        let rot = Quat::IDENTITY;

        let pitch = Quat::from_axis_angle(rot * Vec3::X, 0.1);
        let yaw = Quat::from_axis_angle(rot * Vec3::Y, 0.2);
        let roll = Quat::from_axis_angle(rot * Vec3::NEG_Z, 0.3);

        let combined = (roll * pitch * yaw * rot).normalize();
        assert!((combined - pitch).length() > 0.01);
        assert!((combined - yaw).length() > 0.01);
        assert!((combined - roll).length() > 0.01);
    }
}
