//! Day/night cycle: in-game clock, sun direction, lighting curves.
//!
//! The sun orbits the planet based on a normalized time-of-day value
//! in `[0.0, 1.0)` where 0.0 is midnight, 0.25 is dawn, 0.5 is noon,
//! and 0.75 is dusk. All derived lighting values (intensity, color,
//! ambient, star visibility) update smoothly each frame.

use glam::Vec3;

/// In-game time tracking for the day/night cycle.
#[derive(Clone, Debug)]
pub struct DayNightClock {
    /// Current time of day, normalized `[0.0, 1.0)`. 0.0 = midnight, 0.5 = noon.
    pub time_of_day: f64,
    /// Duration of one full day in real-time seconds.
    pub day_duration_seconds: f64,
    /// Whether the cycle is paused (e.g., in editor mode).
    pub paused: bool,
}

impl DayNightClock {
    /// Create a new clock starting at noon.
    pub fn new(day_duration_seconds: f64) -> Self {
        Self {
            time_of_day: 0.5,
            day_duration_seconds,
            paused: false,
        }
    }

    /// Advance the clock by `dt` real-time seconds.
    pub fn tick(&mut self, dt: f64) {
        if self.paused {
            return;
        }
        let day_fraction = dt / self.day_duration_seconds;
        self.time_of_day = (self.time_of_day + day_fraction) % 1.0;
    }

    /// Convert time-of-day to hours (0–24 range).
    pub fn hours(&self) -> f64 {
        self.time_of_day * 24.0
    }

    /// Returns `true` if currently between civil dawn and civil dusk.
    pub fn is_daytime(&self) -> bool {
        self.time_of_day > 0.2 && self.time_of_day < 0.8
    }
}

/// Compute the sun's direction vector from the time of day.
///
/// The sun orbits in the XZ plane with Y as "up".
/// At `time_of_day = 0.5` (noon) the sun is directly overhead (+Y).
/// At `time_of_day = 0.0` (midnight) the sun is directly below (−Y).
pub fn sun_direction_from_time(time_of_day: f64) -> Vec3 {
    let angle = (time_of_day as f32) * std::f32::consts::TAU;
    Vec3::new(angle.sin(), -angle.cos(), 0.0).normalize()
}

/// Compute the sun intensity multiplier from its elevation.
///
/// Returns a value in `[0.0, 1.0]`:
/// - 1.0 when sun is well above the horizon (elevation > 15°)
/// - 0.0 when sun is well below the horizon (elevation < −10°)
/// - Smooth transition through dawn/dusk
pub fn sun_intensity_curve(sun_direction: Vec3) -> f32 {
    let sin_elevation = sun_direction.y;
    let low = (-10.0_f32).to_radians().sin();
    let high = (15.0_f32).to_radians().sin();
    smoothstep(low, high, sin_elevation)
}

/// Compute ambient light intensity. Higher at night, lower during the day.
pub fn ambient_intensity(sun_direction: Vec3) -> f32 {
    let sun_factor = sun_intensity_curve(sun_direction);
    let night_ambient = 0.05;
    let day_ambient = 0.15;
    lerp(night_ambient, day_ambient, sun_factor)
}

/// Compute the sun's color based on its elevation.
///
/// Returns an RGB color in linear space:
/// - High elevation: warm white `(1.0, 0.98, 0.92)`
/// - Low elevation (dawn/dusk): warm orange `(1.0, 0.6, 0.3)`
/// - Below horizon: fades to black
pub fn sun_color(sun_direction: Vec3) -> Vec3 {
    let intensity = sun_intensity_curve(sun_direction);
    let sin_elevation = sun_direction.y;
    let t = smoothstep(0.0, 0.5, sin_elevation);
    let warm = Vec3::new(1.0, 0.6, 0.3);
    let neutral = Vec3::new(1.0, 0.98, 0.92);
    let color = Vec3::lerp(warm, neutral, t);
    color * intensity
}

/// Compute the opacity of the starfield based on sun intensity.
///
/// Stars are fully visible at night, fully invisible during the day.
pub fn star_visibility(sun_direction: Vec3) -> f32 {
    let sun_factor = sun_intensity_curve(sun_direction);
    (1.0 - sun_factor * 2.0).clamp(0.0, 1.0)
}

/// Aggregate day/night state updated each frame.
#[derive(Clone, Debug)]
pub struct DayNightState {
    /// The in-game clock.
    pub clock: DayNightClock,
    /// Current sun direction (normalized).
    pub sun_direction: Vec3,
    /// Sun color in linear RGB (includes intensity baked in).
    pub sun_color: Vec3,
    /// Sun intensity multiplier `[0.0, 1.0]`.
    pub sun_intensity: f32,
    /// Ambient light intensity.
    pub ambient_intensity: f32,
    /// Star visibility `[0.0, 1.0]`.
    pub star_visibility: f32,
}

impl DayNightState {
    /// Create a new state with the given day duration (real-time seconds).
    pub fn new(day_duration_seconds: f64) -> Self {
        let clock = DayNightClock::new(day_duration_seconds);
        let sun_dir = sun_direction_from_time(clock.time_of_day);
        Self {
            clock,
            sun_direction: sun_dir,
            sun_color: sun_color(sun_dir),
            sun_intensity: sun_intensity_curve(sun_dir),
            ambient_intensity: ambient_intensity(sun_dir),
            star_visibility: star_visibility(sun_dir),
        }
    }

    /// Advance the clock by `dt` seconds and recompute all derived values.
    pub fn tick(&mut self, dt: f64) {
        self.clock.tick(dt);
        self.sun_direction = sun_direction_from_time(self.clock.time_of_day);
        self.sun_intensity = sun_intensity_curve(self.sun_direction);
        self.sun_color = sun_color(self.sun_direction);
        self.ambient_intensity = ambient_intensity(self.sun_direction);
        self.star_visibility = star_visibility(self.sun_direction);
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noon_has_maximum_light() {
        let sun_dir = sun_direction_from_time(0.5);
        let intensity = sun_intensity_curve(sun_dir);
        assert!(
            intensity > 0.95,
            "Noon intensity should be near 1.0, got {intensity}"
        );
        assert!(
            sun_dir.y > 0.9,
            "Noon sun should point upward, got y={}",
            sun_dir.y
        );
    }

    #[test]
    fn test_midnight_has_minimum_light() {
        let sun_dir = sun_direction_from_time(0.0);
        let intensity = sun_intensity_curve(sun_dir);
        assert!(
            intensity < 0.05,
            "Midnight intensity should be near 0.0, got {intensity}"
        );
        assert!(
            sun_dir.y < -0.9,
            "Midnight sun should point downward, got y={}",
            sun_dir.y
        );
    }

    #[test]
    fn test_dawn_dusk_have_warm_colors() {
        for &time in &[0.25, 0.75] {
            let sun_dir = sun_direction_from_time(time);
            let color = sun_color(sun_dir);
            let brightness = color.x + color.y + color.z;
            if brightness > 0.01 {
                assert!(
                    color.x >= color.z,
                    "Dawn/dusk at time {time}: red ({}) should be >= blue ({})",
                    color.x,
                    color.z
                );
            }
        }
    }

    #[test]
    fn test_cycle_duration_is_configurable() {
        let mut clock_fast = DayNightClock::new(60.0);
        let mut clock_slow = DayNightClock::new(3600.0);

        clock_fast.tick(30.0);
        clock_slow.tick(30.0);

        assert!(
            (clock_fast.time_of_day - 0.0).abs() < 0.01,
            "Fast clock should have completed half a day: {}",
            clock_fast.time_of_day
        );

        assert!(
            (clock_slow.time_of_day - 0.508).abs() < 0.01,
            "Slow clock should have barely moved: {}",
            clock_slow.time_of_day
        );
    }

    #[test]
    fn test_lighting_updates_every_frame_smoothly() {
        let mut clock = DayNightClock::new(1200.0);
        let dt = 1.0 / 60.0;

        let mut prev_intensity = sun_intensity_curve(sun_direction_from_time(clock.time_of_day));

        for frame in 0..600 {
            clock.tick(dt);
            let sun_dir = sun_direction_from_time(clock.time_of_day);
            let intensity = sun_intensity_curve(sun_dir);

            let delta = (intensity - prev_intensity).abs();
            assert!(
                delta < 0.01,
                "Frame {frame}: intensity jumped by {delta} (from {prev_intensity} to {intensity})"
            );
            prev_intensity = intensity;
        }
    }

    #[test]
    fn test_star_visibility_at_night() {
        let sun_dir = sun_direction_from_time(0.0);
        let stars = star_visibility(sun_dir);
        assert!(
            stars > 0.9,
            "Stars should be fully visible at midnight, got {stars}"
        );
    }

    #[test]
    fn test_star_visibility_at_noon() {
        let sun_dir = sun_direction_from_time(0.5);
        let stars = star_visibility(sun_dir);
        assert!(
            stars < 0.1,
            "Stars should be invisible at noon, got {stars}"
        );
    }

    #[test]
    fn test_day_night_state_tick() {
        let mut state = DayNightState::new(1200.0);
        let initial_time = state.clock.time_of_day;
        state.tick(10.0);
        assert!(
            (state.clock.time_of_day - initial_time).abs() > 0.0,
            "Clock should advance"
        );
    }

    #[test]
    fn test_clock_paused() {
        let mut clock = DayNightClock::new(1200.0);
        clock.paused = true;
        let before = clock.time_of_day;
        clock.tick(100.0);
        assert!(
            (clock.time_of_day - before).abs() < f64::EPSILON,
            "Paused clock should not advance"
        );
    }

    #[test]
    fn test_hours_conversion() {
        let clock = DayNightClock::new(1200.0);
        assert!((clock.hours() - 12.0).abs() < 0.01, "Noon = 12 hours");
    }

    #[test]
    fn test_is_daytime() {
        let mut clock = DayNightClock::new(1200.0);
        assert!(clock.is_daytime(), "Noon should be daytime");
        clock.time_of_day = 0.0;
        assert!(!clock.is_daytime(), "Midnight should not be daytime");
    }
}
