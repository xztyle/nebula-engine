//! World factory function.

use bevy_ecs::prelude::*;

use crate::TimeRes;

/// Creates and returns a fully initialized ECS world with all engine
/// resources pre-inserted ([`TimeRes`], etc.) and default values.
pub fn create_world() -> World {
    let mut world = World::new();
    world.insert_resource(TimeRes::default());
    world
}
