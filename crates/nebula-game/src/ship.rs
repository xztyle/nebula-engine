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
    /// Planet radius in meters (for gravity and landing calculations).
    pub planet_radius_m: f64,
    /// Surface gravity in m/s² (applied at planet surface, decreases with altitude).
    pub surface_gravity: f64,
    /// Gravity well altitude in meters (gravity only applies below this).
    pub gravity_well_altitude: f64,
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
            planet_radius_m: 6_371_000.0,
            surface_gravity: 9.81,
            gravity_well_altitude: 1_000_000.0,
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
    /// Whether the ship is currently landed on the planet surface.
    pub landed: bool,
    /// Vertical speed relative to planet surface (positive = climbing, m/s).
    pub vertical_speed: f64,
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
            landed: false,
            vertical_speed: 0.0,
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
            landed: false,
            vertical_speed: 0.0,
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

    // --- Planetary gravity ---
    let dist = ship.position.length();
    let altitude = dist - config.planet_radius_m;
    if altitude < config.gravity_well_altitude && dist > 0.0 {
        // g = g_surface * (R / r)² where r = distance from center
        let r = dist.max(config.planet_radius_m);
        let g = config.surface_gravity * (config.planet_radius_m / r).powi(2);
        let gravity_dir = -ship.position.normalize();
        ship.velocity += gravity_dir * g * dt;
    }

    // --- Linear damping (increased near surface for easier landing) ---
    let extra_damping = if altitude < 10_000.0 { 0.2 } else { 0.0 };
    let damping_factor = (1.0 - (config.linear_damping + extra_damping) * dt).max(0.0);
    ship.velocity *= damping_factor;

    // --- Auto-brake below 100m if descending too fast ---
    if altitude < 100.0 && altitude > 0.0 && dist > 0.0 {
        let radial_dir = ship.position.normalize();
        let radial_vel = ship.velocity.dot(radial_dir);
        // If descending faster than 20 m/s, apply retro-braking
        if radial_vel < -20.0 {
            let brake = radial_dir * (radial_vel + 20.0) * 0.5;
            ship.velocity -= brake * dt;
        }
    }

    // --- Integrate position ---
    ship.position += ship.velocity * dt;

    // --- Landing detection ---
    let new_dist = ship.position.length();
    if new_dist <= config.planet_radius_m && new_dist > 0.0 {
        // Clamp to surface
        ship.position = ship.position.normalize() * config.planet_radius_m;
        ship.velocity = DVec3::ZERO;
        ship.landed = true;
    } else if ship.landed && new_dist > config.planet_radius_m {
        ship.landed = false;
    }

    // --- Vertical speed (radial component of velocity) ---
    let current_dist = ship.position.length();
    if current_dist > 0.0 {
        let radial_dir = ship.position.normalize();
        ship.vertical_speed = ship.velocity.dot(radial_dir);
    } else {
        ship.vertical_speed = 0.0;
    }
}

/// Sync the camera to follow the ship (first-person: camera at ship position).
///
/// When `is_boosting` is true, adds subtle screen shake (±0.5m random offset)
/// to give a visceral feel to boost thrust.
pub fn sync_camera_to_ship(camera: &mut Camera, ship: &ShipState, is_boosting: bool) {
    camera.position = Vec3::new(
        ship.position.x as f32,
        ship.position.y as f32,
        ship.position.z as f32,
    );

    // Screen shake during boost: small random offset using a fast hash
    if is_boosting {
        // Simple pseudo-random from position bits (changes every frame)
        let seed = (ship.position.x.to_bits() ^ ship.position.z.to_bits()) as u32;
        let hash = |s: u32| -> f32 {
            let h = s.wrapping_mul(2654435761);
            (h as f32 / u32::MAX as f32) * 2.0 - 1.0
        };
        let shake_amount = 0.5_f32;
        camera.position.x += hash(seed) * shake_amount;
        camera.position.y += hash(seed.wrapping_add(1)) * shake_amount;
        camera.position.z += hash(seed.wrapping_add(2)) * shake_amount;
    }

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
            gravity_well_altitude: 0.0, // disable gravity for this test
            ..ShipConfig::default()
        };
        let mut ship = ShipState::new(DVec3::new(0.0, 10_000_000.0, 0.0));
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
            gravity_well_altitude: 0.0,
            ..ShipConfig::default()
        };
        let mut ship = ShipState::new(DVec3::new(0.0, 10_000_000.0, 0.0));
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
            gravity_well_altitude: 0.0, // disable gravity for this test
            ..ShipConfig::default()
        };
        let mut ship = ShipState::new(DVec3::new(0.0, 10_000_000.0, 0.0));
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
            "Expected ~60m displacement, got {}",
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

    #[test]
    fn test_gravity_pulls_ship_down() {
        let config = ShipConfig::default();
        // Ship 100km above surface, stationary
        let mut ship = ShipState::new(DVec3::new(0.0, 6_471_000.0, 0.0));

        for _ in 0..60 {
            update_ship(
                &mut ship,
                &config,
                1.0 / 60.0,
                &default_kb(),
                &default_mouse(),
            );
        }

        // Should have gained downward velocity from gravity
        assert!(
            ship.velocity.y < -1.0,
            "Ship should be falling, vy={}",
            ship.velocity.y
        );
    }

    #[test]
    fn test_landing_clamps_to_surface() {
        let config = ShipConfig::default();
        // Ship just above surface, falling fast
        let mut ship = ShipState::new(DVec3::new(0.0, 6_371_010.0, 0.0));
        ship.velocity = DVec3::new(0.0, -1000.0, 0.0);

        for _ in 0..60 {
            update_ship(
                &mut ship,
                &config,
                1.0 / 60.0,
                &default_kb(),
                &default_mouse(),
            );
        }

        assert!(ship.landed, "Ship should be landed");
        assert!(
            (ship.position.length() - config.planet_radius_m).abs() < 1.0,
            "Ship should be at surface"
        );
        assert!(
            ship.velocity.length() < 0.01,
            "Velocity should be zero on landing"
        );
    }
}
