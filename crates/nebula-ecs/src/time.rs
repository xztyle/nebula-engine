//! Time resource for the ECS world.

use bevy_ecs::prelude::*;

/// Global time resource inserted into the ECS world at creation.
///
/// Tracks the frame delta time so systems can access timing information
/// without receiving it as a function parameter.
#[derive(Resource, Debug, Clone)]
pub struct TimeRes {
    /// Wall-clock seconds elapsed since the previous frame.
    pub delta: f32,
}

impl Default for TimeRes {
    fn default() -> Self {
        Self { delta: 0.0 }
    }
}
