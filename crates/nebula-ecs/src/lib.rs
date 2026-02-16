//! ECS world setup, schedule definitions, core component types, and system registration utilities.
//!
//! Provides the central [`World`](bevy_ecs::world::World) factory and
//! [`EngineSchedules`] runner that drives the entire engine simulation loop.

mod camera;
mod change_detection;
mod chunk_manager;
mod components;
mod input;
mod lifecycle;
pub mod query_patterns;
mod render_context;
mod schedule;
mod system_ordering;
mod systems;
mod time;
mod voxel_registry;
mod world;

pub use camera::CameraRes;
pub use change_detection::{
    update_all_local_positions_on_camera_move, update_local_positions_incremental,
};
pub use chunk_manager::ChunkManager;
pub use components::{Active, LocalPos, Name, Rotation, Scale, SpatialBundle, Velocity, WorldPos};
pub use input::InputState;
pub use lifecycle::{DespawnQueue, SpawnQueue, despawn_entity, flush_entity_queues, spawn_entity};
pub use query_patterns::{
    KnockbackEffect, MeshHandle, MovementQuery, Simulated, collect_entities, count_entities,
    has_component, movement_system,
};
pub use render_context::RenderContext;
pub use schedule::{EngineSchedule, EngineSchedules};
pub use system_ordering::{
    FixedUpdateSet, PostUpdateSet, PreRenderSet, PreUpdateSet, RenderSet, UpdateSet,
    configure_fixedupdate_ordering, configure_postupdate_ordering, configure_prerender_ordering,
    configure_preupdate_ordering, configure_render_ordering, configure_update_ordering,
    validate_schedules,
};
pub use systems::update_local_positions;
pub use time::TimeRes;
pub use voxel_registry::{VoxelRegistry, VoxelTypeEntry};
pub use world::{create_world, register_core_resources};
