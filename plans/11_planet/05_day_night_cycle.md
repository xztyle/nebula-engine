# Day/Night Cycle

## Problem

A static sun produces a static sky. Players standing on the surface of a planet should experience the passage of time: the sun rises in the east, arcs overhead, sets in the west, and darkness falls as stars appear. Without a day/night cycle, the world feels frozen and lifeless. The cycle drives the entire visual mood of the surface environment -- warm golden light at dawn, harsh white light at noon, deep orange at dusk, and cool blue-black at night. The atmosphere scattering shader (story 04) already responds to the sun direction, but nothing moves the sun. This story rotates the directional light around the planet, smoothly transitioning through all phases of the day, and ties it to an in-game time system so that the cycle duration is configurable (e.g., 20 minutes real-time = one in-game day).

## Solution

### In-Game Time System

Define an in-game clock that advances each frame. The clock tracks the current time-of-day as a normalized value in `[0.0, 1.0)` where 0.0 is midnight, 0.25 is dawn, 0.5 is noon, and 0.75 is dusk:

```rust
/// In-game time tracking for the day/night cycle.
#[derive(Clone, Debug)]
pub struct DayNightClock {
    /// Current time of day, normalized [0.0, 1.0). 0.0 = midnight, 0.5 = noon.
    pub time_of_day: f64,
    /// Duration of one full day in real-time seconds.
    pub day_duration_seconds: f64,
    /// Whether the cycle is paused (e.g., in editor mode).
    pub paused: bool,
}

impl DayNightClock {
    pub fn new(day_duration_seconds: f64) -> Self {
        Self {
            time_of_day: 0.5, // Start at noon
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

    /// Convert time_of_day to hours (0-24 range).
    pub fn hours(&self) -> f64 {
        self.time_of_day * 24.0
    }

    /// Returns true if currently between civil dawn and civil dusk.
    pub fn is_daytime(&self) -> bool {
        self.time_of_day > 0.2 && self.time_of_day < 0.8
    }
}
```

### Sun Direction from Time

The sun direction is computed by rotating around the planet's axis. The rotation angle is derived directly from `time_of_day`:

```rust
use glam::Vec3;

/// Compute the sun's direction vector from the time of day.
///
/// The sun orbits in the XY plane (east-west), with Y as "up" on the planet.
/// At time_of_day = 0.5 (noon), the sun is directly overhead (+Y).
/// At time_of_day = 0.0 (midnight), the sun is directly below (-Y).
pub fn sun_direction_from_time(time_of_day: f64) -> Vec3 {
    let angle = (time_of_day as f32) * std::f32::consts::TAU;
    // Rotate in the XZ plane (sun rises in +X, sets in -X).
    // At angle=0 (midnight), sun is at -Y. At angle=PI (noon), sun is at +Y.
    Vec3::new(
        angle.sin(),
        -angle.cos(), // -cos so that 0.0 = nadir, 0.5 = zenith
        0.0,
    )
    .normalize()
}
```

### Light Intensity Curve

The sun's intensity does not switch instantly between day and night. Instead, it follows a smooth curve that ramps up at dawn and down at dusk:

```rust
/// Compute the sun intensity multiplier based on the sun's elevation angle.
///
/// Returns a value in [0.0, 1.0]:
/// - 1.0 when sun is well above the horizon (elevation > 15°)
/// - 0.0 when sun is well below the horizon (elevation < -10°)
/// - Smooth transition through dawn/dusk
pub fn sun_intensity_curve(sun_direction: Vec3) -> f32 {
    // sin_elevation = sun_direction.y (dot with up vector).
    let sin_elevation = sun_direction.y;

    // Smooth hermite interpolation between -10° and +15° elevation.
    let low = (-10.0_f32).to_radians().sin();   // ~-0.174
    let high = (15.0_f32).to_radians().sin();    // ~0.259

    smoothstep(low, high, sin_elevation)
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
```

### Ambient Light at Night

When the sun is below the horizon, ambient light provides a minimum illumination level (moonlight, starlight). The ambient intensity is the inverse of the sun intensity curve, scaled to a low baseline:

```rust
/// Compute ambient light intensity. Higher at night, lower during the day.
pub fn ambient_intensity(sun_direction: Vec3) -> f32 {
    let sun_factor = sun_intensity_curve(sun_direction);
    let night_ambient = 0.05; // Dim moonlight
    let day_ambient = 0.15;   // Ambient fill during the day
    lerp(night_ambient, day_ambient, sun_factor)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
```

### Light Color Temperature

The sun's color temperature shifts throughout the day. Noon light is neutral white. Dawn and dusk light is warm orange. The color is driven by the elevation angle:

```rust
/// Compute the sun's color based on its elevation.
///
/// Returns an RGB color in linear space:
/// - High elevation: warm white (1.0, 0.98, 0.92)
/// - Low elevation (dawn/dusk): warm orange (1.0, 0.6, 0.3)
/// - Below horizon: fades to black
pub fn sun_color(sun_direction: Vec3) -> Vec3 {
    let intensity = sun_intensity_curve(sun_direction);
    let sin_elevation = sun_direction.y;

    // Blend between warm orange (near horizon) and neutral white (high elevation).
    let t = smoothstep(0.0, 0.5, sin_elevation);
    let warm = Vec3::new(1.0, 0.6, 0.3);
    let neutral = Vec3::new(1.0, 0.98, 0.92);
    let color = Vec3::lerp(warm, neutral, t);

    color * intensity
}
```

### Star Visibility

Stars (from the space rendering system, Epic 12) become visible as the sun intensity drops. The star alpha is the inverse of the sun intensity:

```rust
/// Compute the opacity of the starfield based on sun intensity.
/// Stars are fully visible at night, fully invisible during the day.
pub fn star_visibility(sun_direction: Vec3) -> f32 {
    let sun_factor = sun_intensity_curve(sun_direction);
    // Fade stars out as sun rises. Stars disappear once sun is ~5° above horizon.
    (1.0 - sun_factor * 2.0).clamp(0.0, 1.0)
}
```

### Per-Frame Update System

The day/night cycle is driven by a Bevy ECS system that runs every frame:

```rust
use bevy_ecs::prelude::*;

#[derive(Resource)]
pub struct DayNightState {
    pub clock: DayNightClock,
    pub sun_direction: Vec3,
    pub sun_color: Vec3,
    pub sun_intensity: f32,
    pub ambient_intensity: f32,
    pub star_visibility: f32,
}

pub fn update_day_night_cycle(
    time: Res<Time>,
    mut state: ResMut<DayNightState>,
) {
    state.clock.tick(time.delta_secs_f64());

    state.sun_direction = sun_direction_from_time(state.clock.time_of_day);
    state.sun_intensity = sun_intensity_curve(state.sun_direction);
    state.sun_color = sun_color(state.sun_direction);
    state.ambient_intensity = ambient_intensity(state.sun_direction);
    state.star_visibility = star_visibility(state.sun_direction);
}
```

## Outcome

The `nebula-planet` crate exports `DayNightClock`, `DayNightState`, and the `update_day_night_cycle` system. Each frame, the sun direction is recomputed from the in-game clock, and all derived lighting values (intensity, color, ambient, star visibility) are updated. The atmosphere scattering shader (story 04) reads the updated sun direction and produces the correct sky color for the current time of day. The cycle duration is configurable and defaults to 20 minutes of real time per in-game day.

## Demo Integration

**Demo crate:** `nebula-demo`

The sun moves across the sky over a 20-minute real-time cycle. Dawn and dusk produce warm orange/pink sky colors. Night reveals stars. The lighting transitions smoothly.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | `0.29` | Vec3 sun direction, color math |
| `bevy_ecs` | `0.16` | ECS system scheduling, Time resource |

Internal dependencies: `nebula-lighting` (directional light integration). Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    const EPSILON: f32 = 1e-4;

    #[test]
    fn test_noon_has_maximum_light() {
        let sun_dir = sun_direction_from_time(0.5); // noon
        let intensity = sun_intensity_curve(sun_dir);
        assert!(
            intensity > 0.95,
            "Noon intensity should be near 1.0, got {intensity}"
        );
        // Sun should be pointing upward at noon.
        assert!(
            sun_dir.y > 0.9,
            "Noon sun should point upward, got y={}",
            sun_dir.y
        );
    }

    #[test]
    fn test_midnight_has_minimum_light() {
        let sun_dir = sun_direction_from_time(0.0); // midnight
        let intensity = sun_intensity_curve(sun_dir);
        assert!(
            intensity < 0.05,
            "Midnight intensity should be near 0.0, got {intensity}"
        );
        // Sun should be pointing downward at midnight.
        assert!(
            sun_dir.y < -0.9,
            "Midnight sun should point downward, got y={}",
            sun_dir.y
        );
    }

    #[test]
    fn test_dawn_dusk_have_warm_colors() {
        // Dawn at ~0.25 (6:00 AM), dusk at ~0.75 (6:00 PM).
        for &time in &[0.25, 0.75] {
            let sun_dir = sun_direction_from_time(time);
            let color = sun_color(sun_dir);

            // At dawn/dusk, the sun is near the horizon. If there is any light,
            // red should be stronger relative to blue (warm tone).
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
        let mut clock_fast = DayNightClock::new(60.0);   // 60 seconds per day
        let mut clock_slow = DayNightClock::new(3600.0);  // 1 hour per day

        // Advance both by 30 real seconds.
        clock_fast.tick(30.0);
        clock_slow.tick(30.0);

        // Fast clock should be at 0.5 + 30/60 = 1.0 => wraps to ~0.0.
        assert!(
            (clock_fast.time_of_day - 0.0).abs() < 0.01,
            "Fast clock should have completed half a day: {}",
            clock_fast.time_of_day
        );

        // Slow clock should barely have moved: 0.5 + 30/3600 ≈ 0.508.
        assert!(
            (clock_slow.time_of_day - 0.508).abs() < 0.01,
            "Slow clock should have barely moved: {}",
            clock_slow.time_of_day
        );
    }

    #[test]
    fn test_lighting_updates_every_frame_smoothly() {
        let mut clock = DayNightClock::new(1200.0); // 20 minutes
        let dt = 1.0 / 60.0; // 60 FPS

        let mut prev_intensity = sun_intensity_curve(
            sun_direction_from_time(clock.time_of_day),
        );

        // Simulate 600 frames (~10 seconds real time).
        for frame in 0..600 {
            clock.tick(dt);
            let sun_dir = sun_direction_from_time(clock.time_of_day);
            let intensity = sun_intensity_curve(sun_dir);

            let delta = (intensity - prev_intensity).abs();
            // At 60 FPS over a 20-minute cycle, each frame represents
            // 1/72000 of the cycle. The intensity change per frame should be tiny.
            assert!(
                delta < 0.01,
                "Frame {frame}: intensity jumped by {delta} (from {prev_intensity} to {intensity})"
            );
            prev_intensity = intensity;
        }
    }

    #[test]
    fn test_star_visibility_at_night() {
        let sun_dir = sun_direction_from_time(0.0); // midnight
        let stars = star_visibility(sun_dir);
        assert!(
            stars > 0.9,
            "Stars should be fully visible at midnight, got {stars}"
        );
    }

    #[test]
    fn test_star_visibility_at_noon() {
        let sun_dir = sun_direction_from_time(0.5); // noon
        let stars = star_visibility(sun_dir);
        assert!(
            stars < 0.1,
            "Stars should be invisible at noon, got {stars}"
        );
    }
}
```
