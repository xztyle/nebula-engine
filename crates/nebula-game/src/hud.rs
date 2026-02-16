//! Basic HUD overlay displayed via the window title.
//!
//! Computes speed, altitude, throttle, heading, and FPS from the ship state
//! and formats them as a compact string for the window title bar.

use crate::ship::ShipState;
use nebula_planet::TransitionConfig;
use std::time::Instant;
use tracing::info;

/// HUD telemetry values computed each frame.
#[derive(Debug, Clone)]
pub struct HudState {
    /// Ship speed in meters per second.
    pub speed_mps: f64,
    /// Altitude above planet surface in meters.
    pub altitude_m: f64,
    /// Throttle percentage (0 = idle, 100 = full, up to 1000 = boost).
    pub throttle_pct: f64,
    /// Heading in degrees (0–360, 0 = north/+Z).
    pub heading_deg: f64,
    /// Frames per second (smoothed).
    pub fps: f64,
    /// Transition mode label (e.g. "Orbital", "Blend", "Surface").
    pub transition_mode: &'static str,
    /// Orbital sphere blend factor (0 = surface, 1 = orbital).
    pub transition_blend: f32,
    /// Last frame timestamp for FPS calculation.
    last_frame: Instant,
    /// Exponential moving average of frame time.
    frame_time_ema: f64,
    /// Previous transition mode for logging state changes.
    prev_transition_mode: &'static str,
    /// Whether the ship is landed on the surface.
    pub landed: bool,
    /// Whether the ship is in landing mode (altitude < 1 km).
    pub landing_mode: bool,
    /// Vertical speed in m/s (positive = climbing).
    pub vertical_speed: f64,
}

impl Default for HudState {
    fn default() -> Self {
        Self {
            speed_mps: 0.0,
            altitude_m: 0.0,
            throttle_pct: 0.0,
            heading_deg: 0.0,
            fps: 0.0,
            transition_mode: "Orbital",
            transition_blend: 1.0,
            last_frame: Instant::now(),
            frame_time_ema: 1.0 / 60.0,
            prev_transition_mode: "Orbital",
            landed: false,
            landing_mode: false,
            vertical_speed: 0.0,
        }
    }
}

/// Update HUD values from the current ship state and planet radius.
///
/// Call once per simulation tick. `is_thrusting` indicates whether any
/// thrust key is held, and `is_boosting` indicates the boost modifier.
pub fn update_hud(
    hud: &mut HudState,
    ship: &ShipState,
    planet_radius_m: f64,
    is_thrusting: bool,
    is_boosting: bool,
) {
    // Speed: magnitude of velocity vector
    hud.speed_mps = ship.velocity.length();

    // Altitude: distance from origin minus planet radius
    let dist = ship.position.length();
    hud.altitude_m = (dist - planet_radius_m).max(0.0);

    // Throttle: 0% idle, 100% thrusting, 1000% boosting
    hud.throttle_pct = if is_boosting {
        1000.0
    } else if is_thrusting {
        100.0
    } else {
        0.0
    };

    // Heading: extract yaw from orientation quaternion.
    // Forward is -Z in ship space; project onto the XZ plane.
    let forward = ship.orientation * -glam::DVec3::Z;
    // atan2(x, z) gives angle from +Z axis (north), clockwise when viewed from above.
    let yaw_rad = forward.x.atan2(forward.z);
    hud.heading_deg = yaw_rad.to_degrees().rem_euclid(360.0);

    // FPS: exponential moving average
    let now = Instant::now();
    let dt = now.duration_since(hud.last_frame).as_secs_f64();
    hud.last_frame = now;
    if dt > 0.0 {
        // EMA with α = 0.1 for smooth display
        hud.frame_time_ema = hud.frame_time_ema * 0.9 + dt * 0.1;
        hud.fps = 1.0 / hud.frame_time_ema;
    }

    // Landing state
    hud.landed = ship.landed;
    hud.landing_mode = hud.altitude_m < 1_000.0 && !ship.landed;
    hud.vertical_speed = ship.vertical_speed;

    // Transition state from altitude
    let transition = TransitionConfig::default();
    let (_mode, blend) = transition.classify(hud.altitude_m);
    hud.transition_blend = blend;
    hud.transition_mode = if blend >= 1.0 {
        "Orbital"
    } else if blend <= 0.0 {
        "Surface"
    } else {
        "Blend"
    };

    // Log transition state changes
    if hud.transition_mode != hud.prev_transition_mode {
        info!(
            "Transition: {} -> {} (alt={:.1}km, blend={:.2})",
            hud.prev_transition_mode,
            hud.transition_mode,
            hud.altitude_m / 1000.0,
            blend,
        );
        hud.prev_transition_mode = hud.transition_mode;
    }
}

/// Format HUD values as a compact string suitable for a window title.
///
/// Example: `SPD: 1,234 m/s | ALT: 402.3 km | THR: 75% | HDG: 045° | FPS: 144`
pub fn format_hud(hud: &HudState) -> String {
    let throttle = hud.throttle_pct;
    let heading = hud.heading_deg;
    let fps = hud.fps;

    // Speed formatting: km/s when > 1000 m/s
    let speed_str = if hud.speed_mps > 1000.0 {
        format!("{:.1} km/s", hud.speed_mps / 1000.0)
    } else {
        format!("{} m/s", format_with_commas(hud.speed_mps as u64))
    };

    // Altitude formatting: Mm when > 1000 km, km otherwise
    let alt_str = if hud.altitude_m > 1_000_000.0 {
        format!("{:.2} Mm", hud.altitude_m / 1_000_000.0)
    } else {
        format!("{:.1} km", hud.altitude_m / 1000.0)
    };

    let transition = hud.transition_mode;
    let blend_pct = hud.transition_blend * 100.0;

    // Supercruise indicator at high speed
    let flight_mode = if hud.speed_mps > 2000.0 {
        " | SUPERCRUISE"
    } else {
        ""
    };

    // Landing/vertical speed indicators
    let landing_str = if hud.landed {
        " | SURFACE | Press SPACE to launch".to_string()
    } else if hud.landing_mode {
        format!(" | LANDING MODE | VS: {:.1} m/s", hud.vertical_speed)
    } else if hud.altitude_m < 10_000.0 {
        format!(" | VS: {:.1} m/s", hud.vertical_speed)
    } else {
        String::new()
    };

    format!(
        "SPD: {speed_str} | ALT: {alt_str} | THR: {throttle:.0}% | HDG: {heading:03.0}\u{00b0} | {transition}({blend_pct:.0}%){flight_mode}{landing_str} | FPS: {fps:.0}",
    )
}

/// Format an integer with comma thousands separators.
fn format_with_commas(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ship::ShipState;
    use glam::{DQuat, DVec3};

    fn make_ship(pos: DVec3, vel: DVec3, orientation: DQuat) -> ShipState {
        ShipState {
            position: pos,
            velocity: vel,
            orientation,
            angular_velocity: DVec3::ZERO,
            landed: false,
            vertical_speed: 0.0,
        }
    }

    #[test]
    fn test_speed_from_velocity() {
        let ship = make_ship(
            DVec3::new(0.0, 7_000_000.0, 0.0),
            DVec3::new(100.0, 0.0, 0.0),
            DQuat::IDENTITY,
        );
        let mut hud = HudState::default();
        update_hud(&mut hud, &ship, 6_371_000.0, false, false);
        assert!((hud.speed_mps - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_altitude_above_planet() {
        let ship = make_ship(
            DVec3::new(0.0, 6_771_000.0, 0.0),
            DVec3::ZERO,
            DQuat::IDENTITY,
        );
        let mut hud = HudState::default();
        update_hud(&mut hud, &ship, 6_371_000.0, false, false);
        assert!((hud.altitude_m - 400_000.0).abs() < 1.0);
    }

    #[test]
    fn test_throttle_states() {
        let ship = make_ship(DVec3::Y * 7_000_000.0, DVec3::ZERO, DQuat::IDENTITY);
        let mut hud = HudState::default();

        update_hud(&mut hud, &ship, 6_371_000.0, false, false);
        assert!((hud.throttle_pct - 0.0).abs() < f64::EPSILON);

        update_hud(&mut hud, &ship, 6_371_000.0, true, false);
        assert!((hud.throttle_pct - 100.0).abs() < f64::EPSILON);

        update_hud(&mut hud, &ship, 6_371_000.0, true, true);
        assert!((hud.throttle_pct - 1000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_heading_north() {
        // Identity quaternion: forward is -Z, heading should be 180° (facing south)
        let ship = make_ship(DVec3::Y * 7_000_000.0, DVec3::ZERO, DQuat::IDENTITY);
        let mut hud = HudState::default();
        update_hud(&mut hud, &ship, 6_371_000.0, false, false);
        // -Z forward → atan2(0, -1) = π = 180°
        assert!((hud.heading_deg - 180.0).abs() < 0.1);
    }

    #[test]
    fn test_format_with_commas() {
        assert_eq!(format_with_commas(0), "0");
        assert_eq!(format_with_commas(999), "999");
        assert_eq!(format_with_commas(1000), "1,000");
        assert_eq!(format_with_commas(1_234_567), "1,234,567");
    }

    #[test]
    fn test_format_hud_output() {
        let hud = HudState {
            speed_mps: 1234.0,
            altitude_m: 402_300.0,
            throttle_pct: 75.0,
            heading_deg: 45.0,
            fps: 144.0,
            transition_mode: "Orbital",
            transition_blend: 1.0,
            ..HudState::default()
        };
        let s = format_hud(&hud);
        assert!(s.contains("SPD: 1.2 km/s"), "got: {s}");
        assert!(s.contains("ALT: 402.3 km"));
        assert!(s.contains("THR: 75%"));
        assert!(s.contains("HDG: 045°"));
        assert!(s.contains("Orbital(100%)"));
        assert!(s.contains("FPS: 144"));
    }

    #[test]
    fn test_format_hud_megameters() {
        let hud = HudState {
            speed_mps: 500.0,
            altitude_m: 2_500_000.0,
            throttle_pct: 0.0,
            heading_deg: 0.0,
            fps: 60.0,
            transition_mode: "Orbital",
            transition_blend: 1.0,
            ..HudState::default()
        };
        let s = format_hud(&hud);
        assert!(s.contains("ALT: 2.50 Mm"), "got: {s}");
        assert!(s.contains("SPD: 500 m/s"), "got: {s}");
    }

    #[test]
    fn test_format_hud_supercruise() {
        let hud = HudState {
            speed_mps: 3000.0,
            altitude_m: 100_000.0,
            ..HudState::default()
        };
        let s = format_hud(&hud);
        assert!(s.contains("SUPERCRUISE"), "got: {s}");
    }

    #[test]
    fn test_format_hud_landed() {
        let hud = HudState {
            landed: true,
            ..HudState::default()
        };
        let s = format_hud(&hud);
        assert!(s.contains("SURFACE | Press SPACE to launch"), "got: {s}");
    }
}
