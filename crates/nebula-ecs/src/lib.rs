//! ECS world setup, schedule definitions, core component types, and system registration utilities.
//!
//! Provides the central [`World`](bevy_ecs::world::World) factory and
//! [`EngineSchedules`] runner that drives the entire engine simulation loop.

mod components;
mod schedule;
mod time;
mod world;

pub use components::{Active, LocalPos, Name, Rotation, Scale, SpatialBundle, Velocity, WorldPos};
pub use schedule::{EngineSchedule, EngineSchedules};
pub use time::TimeRes;
pub use world::create_world;
