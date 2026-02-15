//! i128/u128 vector types, fixed-point arithmetic, and fundamental math operations for the Nebula Engine.

mod vector;
mod world_position;

pub use vector::{Vec2I128, Vec3I128, distance_f64, distance_squared, manhattan_distance};
pub use world_position::WorldPosition;
