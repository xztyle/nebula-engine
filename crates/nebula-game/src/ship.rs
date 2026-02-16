//! Ship entity with 6DOF Newtonian flight model.
//!
//! Provides [`ShipState`] for position/velocity/orientation tracking and
//! [`ShipConfig`] for tuning thrust, rotation, and damping parameters.
//! The [`update_ship`] function processes keyboard/mouse input each tick
//! and applies Newtonian physics: thrust adds to velocity, velocity persists,
//! and configurable linear damping prevents pure ice-skating.

use glam::{DQuat, DVec3, Quat, Vec3};
use nebula_input::{KeyboardState, MouseState};
use nebula_render::Camera;
use winit::keyboard::{KeyCode, PhysicalKey};

/// Ship physics configuration.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ShipConfig {
    /// Maximum main thrust force in Newtons.
    pub max_thrust: f64,
    /// Boost thrust multiplier (applied when Ctrl is held).
    pub boost_multiplier: f64,
    /// Maximum rotation speed in radians per second.
    pub max_rotation_speed: f64,
    /// Linear damping coefficient (0 = no damping, 1 = heavy damping).
    /// Applied as `velocity *= (1 - damping * dt)` each tick.
    pub linear_damping: f64,
    /// Angular damping coefficient for rotation decay.
    pub angular_damping: f64,
    /// Ship mass in kilograms (affects acceleration from thrust).
    pub mass: f64,
    /// Mouse look sensitivity (radians per pixel of mouse delta).
    pub mouse_sensitivity: f64,
    /// Roll speed in radians per second when Q/E are held.
    pub roll_speed: f64,
}

impl Default for ShipConfig {
    fn default() -> Self {
        Self {
            // 500 m/s cruise with ~2s to reach speed at 1000kg mass
            max_thrust: 500_000.0,
            boost_multiplier: 10.0,
            max_rotation_speed: 2.0,
            linear_damping: 0.05,
            angular_damping: 3.0,
            mass: 1000.0,
            mouse_sensitivity: 0.003,
            roll_speed: 1.5,
        }
    }
}

/// Runtime ship state: position, velocity, orientation, and angular velocity.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ShipState {
    /// Position in world space (meters, f64 for planetary-scale precision).
    pub position: DVec3,
    /// Velocity in world space (meters per second).
    pub velocity: DVec3,
    /// Orientation as a unit quaternion.
    pub orientation: DQuat,
    /// Angular velocity (pitch, yaw, roll) in radians per second, ship-local frame.
    pub angular_velocity: DVec3,
}

impl ShipState {
    /// Create a new ship at the given position, looking in the -Z direction.
    #[cfg(test)]
    pub fn new(position: DVec3) -> Self {
        Self {
            position,
            velocity: DVec3::ZERO,
            orientation: DQuat::IDENTITY,
            angular_velocity: DVec3::ZERO,
        }
    }

    /// Create a ship at orbital altitude above the planet, matching the camera start.
    pub fn at_orbit(planet_radius: f64, altitude: f64) -> Self {
        let pos = DVec3::new(0.0, planet_radius + altitude, 0.0);
        // Look down toward the planet (rotate -90° around X)
        let orientation = DQuat::from_rotation_x(-std::f64::consts::FRAC_PI_2);
        Self {
            position: pos,
            velocity: DVec3::ZERO,
            orientation,
            angular_velocity: DVec3::ZERO,
        }
    }

    /// Current speed in meters per second.
    #[cfg(test)]
    pub fn speed(&self) -> f64 {
        self.velocity.length()
    }

    /// Forward direction in world space (ship's -Z axis).
    pub fn forward(&self) -> DVec3 {
        self.orientation * -DVec3::Z
    }

    /// Right direction in world space (ship's +X axis).
    pub fn right(&self) -> DVec3 {
        self.orientation * DVec3::X
    }

    /// Up direction in world space (ship's +Y axis).
    pub fn up(&self) -> DVec3 {
        self.orientation * DVec3::Y
    }
}

/// Helper to check if a key is pressed.
fn key_held(kb: &KeyboardState, code: KeyCode) -> bool {
    kb.is_pressed(PhysicalKey::Code(code))
}

/// Update ship physics for one simulation tick.
///
/// Reads keyboard for thrust (WASD, Space/Shift) and boost (Ctrl),
/// reads mouse delta for yaw/pitch, Q/E for roll.
/// Applies Newtonian physics with configurable damping.
pub fn update_ship(
    ship: &mut ShipState,
    config: &ShipConfig,
    dt: f64,
    keyboard: &KeyboardState,
    mouse: &MouseState,
) {
    // --- Rotation from mouse + Q/E roll ---
    let mouse_delta = mouse.delta();
    let yaw = -(mouse_delta.x as f64) * config.mouse_sensitivity;
    let pitch = -(mouse_delta.y as f64) * config.mouse_sensitivity;

    let mut roll = 0.0;
    if key_held(keyboard, KeyCode::KeyQ) {
        roll += config.roll_speed * dt;
    }
    if key_held(keyboard, KeyCode::KeyE) {
        roll -= config.roll_speed * dt;
    }

    // Apply rotation directly (not through angular velocity for mouse look)
    let yaw_quat = DQuat::from_axis_angle(ship.up(), yaw);
    let pitch_quat = DQuat::from_axis_angle(ship.right(), pitch);
    let roll_quat = DQuat::from_axis_angle(ship.forward(), roll);
    ship.orientation = (yaw_quat * pitch_quat * roll_quat * ship.orientation).normalize();

    // --- Thrust from keyboard ---
    let mut thrust_local = DVec3::ZERO;

    // W/S: forward/backward
    if key_held(keyboard, KeyCode::KeyW) {
        thrust_local.z -= 1.0; // forward is -Z in local space
    }
    if key_held(keyboard, KeyCode::KeyS) {
        thrust_local.z += 1.0;
    }

    // A/D: strafe left/right
    if key_held(keyboard, KeyCode::KeyA) {
        thrust_local.x -= 1.0;
    }
    if key_held(keyboard, KeyCode::KeyD) {
        thrust_local.x += 1.0;
    }

    // Space/Shift: vertical thrust
    if key_held(keyboard, KeyCode::Space) {
        thrust_local.y += 1.0;
    }
    if key_held(keyboard, KeyCode::ShiftLeft) {
        thrust_local.y -= 1.0;
    }

    // Normalize so diagonal thrust isn't stronger
    if thrust_local.length_squared() > 0.0 {
        thrust_local = thrust_local.normalize();
    }

    // Boost with Ctrl
    let thrust_magnitude = if key_held(keyboard, KeyCode::ControlLeft) {
        config.max_thrust * config.boost_multiplier
    } else {
        config.max_thrust
    };

    // Transform thrust to world space and apply F = ma
    let thrust_world = ship.orientation * (thrust_local * thrust_magnitude);
    let acceleration = thrust_world / config.mass;
    ship.velocity += acceleration * dt;

    // --- Linear damping ---
    let damping_factor = (1.0 - config.linear_damping * dt).max(0.0);
    ship.velocity *= damping_factor;

    // --- Integrate position ---
    ship.position += ship.velocity * dt;
}

/// Sync the camera to follow the ship (first-person: camera at ship position).
pub fn sync_camera_to_ship(camera: &mut Camera, ship: &ShipState) {
    camera.position = Vec3::new(
        ship.position.x as f32,
        ship.position.y as f32,
        ship.position.z as f32,
    );
    camera.rotation = Quat::from_xyzw(
        ship.orientation.x as f32,
        ship.orientation.y as f32,
        ship.orientation.z as f32,
        ship.orientation.w as f32,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_kb() -> KeyboardState {
        KeyboardState::new()
    }

    fn default_mouse() -> MouseState {
        MouseState::new()
    }

    #[test]
    fn test_ship_starts_stationary() {
        let ship = ShipState::new(DVec3::new(0.0, 100.0, 0.0));
        assert_eq!(ship.velocity, DVec3::ZERO);
        assert!((ship.speed() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_no_input_no_acceleration() {
        let config = ShipConfig {
            linear_damping: 0.0,
            ..ShipConfig::default()
        };
        let mut ship = ShipState::new(DVec3::ZERO);
        ship.velocity = DVec3::new(100.0, 0.0, 0.0);
        let initial_speed = ship.speed();

        update_ship(
            &mut ship,
            &config,
            1.0 / 60.0,
            &default_kb(),
            &default_mouse(),
        );

        // Velocity should be preserved (no damping, no thrust)
        assert!(
            (ship.speed() - initial_speed).abs() < 0.01,
            "Speed should be preserved, got {}",
            ship.speed()
        );
    }

    #[test]
    fn test_thrust_increases_velocity() {
        let config = ShipConfig {
            linear_damping: 0.0,
            ..ShipConfig::default()
        };
        let mut ship = ShipState::new(DVec3::ZERO);

        // Simulate W key held for 60 ticks
        let mut kb = KeyboardState::new();
        use winit::event::ElementState;
        kb.process_raw(nebula_input::keyboard::RawKeyEvent {
            key: PhysicalKey::Code(KeyCode::KeyW),
            state: ElementState::Pressed,
            repeat: false,
        });

        for _ in 0..60 {
            update_ship(&mut ship, &config, 1.0 / 60.0, &kb, &default_mouse());
        }

        assert!(ship.speed() > 0.0, "Ship should be moving after thrust");
        // Forward is -Z, so velocity should be in -Z direction
        let fwd = ship.forward();
        let dot = ship.velocity.normalize().dot(fwd);
        assert!(dot > 0.9, "Velocity should be roughly forward, dot={dot}");
    }

    #[test]
    fn test_damping_slows_ship() {
        let config = ShipConfig {
            linear_damping: 1.0,
            ..ShipConfig::default()
        };
        let mut ship = ShipState::new(DVec3::ZERO);
        ship.velocity = DVec3::new(100.0, 0.0, 0.0);

        for _ in 0..600 {
            update_ship(
                &mut ship,
                &config,
                1.0 / 60.0,
                &default_kb(),
                &default_mouse(),
            );
        }

        assert!(
            ship.speed() < 1.0,
            "Ship should be nearly stopped with heavy damping, got {}",
            ship.speed()
        );
    }

    #[test]
    fn test_position_integrates_from_velocity() {
        let config = ShipConfig {
            linear_damping: 0.0,
            ..ShipConfig::default()
        };
        let mut ship = ShipState::new(DVec3::ZERO);
        ship.velocity = DVec3::new(60.0, 0.0, 0.0);

        // 60 ticks at 1/60s = 1 second, velocity 60 m/s = 60m traveled
        for _ in 0..60 {
            update_ship(
                &mut ship,
                &config,
                1.0 / 60.0,
                &default_kb(),
                &default_mouse(),
            );
        }

        assert!(
            (ship.position.x - 60.0).abs() < 1.0,
            "Expected ~60m, got {}",
            ship.position.x
        );
    }

    #[test]
    fn test_orbital_start_position() {
        let ship = ShipState::at_orbit(6_371_000.0, 400_000.0);
        assert!(
            (ship.position.y - 6_771_000.0).abs() < 1.0,
            "Expected 6771km altitude, got {}",
            ship.position.y
        );
    }
}
