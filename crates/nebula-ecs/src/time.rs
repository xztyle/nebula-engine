//! Time resource for the ECS world.

use bevy_ecs::prelude::*;

/// Global time resource inserted into the ECS world at creation.
///
/// Tracks the frame delta time, total elapsed time, and tick count so
/// systems can access timing information without receiving it as a
/// function parameter.
#[derive(Resource, Debug, Clone)]
pub struct TimeRes {
    /// Wall-clock seconds elapsed since the previous frame.
    pub delta: f32,
    /// Total elapsed time in seconds since engine start.
    pub elapsed: f64,
    /// Monotonic tick counter incremented once per frame in PreUpdate.
    pub tick: u64,
    /// Fixed timestep delta in seconds (default 1/60).
    pub fixed_dt: f64,
}

impl Default for TimeRes {
    fn default() -> Self {
        Self {
            delta: 0.0,
            elapsed: 0.0,
            tick: 0,
            fixed_dt: 1.0 / 60.0,
        }
    }
}
