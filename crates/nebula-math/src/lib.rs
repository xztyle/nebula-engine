//! i128/u128 vector types, fixed-point arithmetic, and fundamental math operations for the Nebula Engine.

mod aabb;
mod conversion;
mod fixed_point;
mod local_position;
mod vector;
mod world_position;

pub use aabb::Aabb128;
pub use conversion::{MAX_SAFE_LOCAL_DELTA, to_local, to_local_batch, to_local_checked, to_world};
pub use fixed_point::FixedI128;
pub use local_position::LocalPosition;
pub use vector::{Vec2I128, Vec3I128, distance_f64, distance_squared, manhattan_distance};
pub use world_position::WorldPosition;
