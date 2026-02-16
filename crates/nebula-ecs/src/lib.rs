//! ECS world setup, schedule definitions, core component types, and system registration utilities.
//!
//! Provides the central [`World`](bevy_ecs::world::World) factory and
//! [`EngineSchedules`] runner that drives the entire engine simulation loop.

mod camera;
mod change_detection;
mod components;
mod lifecycle;
mod schedule;
mod systems;
mod time;
mod world;

pub use camera::CameraRes;
pub use change_detection::{
    update_all_local_positions_on_camera_move, update_local_positions_incremental,
};
pub use components::{Active, LocalPos, Name, Rotation, Scale, SpatialBundle, Velocity, WorldPos};
pub use lifecycle::{DespawnQueue, SpawnQueue, despawn_entity, flush_entity_queues, spawn_entity};
pub use schedule::{EngineSchedule, EngineSchedules};
pub use systems::update_local_positions;
pub use time::TimeRes;
pub use world::create_world;
