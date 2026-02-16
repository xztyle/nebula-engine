//! i128/u128 vector types, fixed-point arithmetic, and fundamental math operations for the Nebula Engine.

mod aabb;
mod conversion;
mod fixed_point;
mod local_position;
mod units;
mod vector;
mod world_position;

pub use aabb::Aabb128;
pub use conversion::{MAX_SAFE_LOCAL_DELTA, to_local, to_local_batch, to_local_checked, to_world};
pub use fixed_point::FixedI128;
pub use local_position::LocalPosition;
pub use units::{
    EARTH_RADIUS_UNITS, SOLAR_RADIUS_UNITS, UNITS_PER_AU, UNITS_PER_CENTIMETER, UNITS_PER_INCH,
    UNITS_PER_KILOMETER, UNITS_PER_LIGHT_YEAR, UNITS_PER_METER, UNITS_PER_PARSEC, au_to_units,
    format_distance, kilometers_to_units, light_years_to_units, meters_to_units, units_to_au,
    units_to_kilometers, units_to_light_years, units_to_meters,
};
pub use vector::{Vec2I128, Vec3I128, distance_f64, distance_squared, manhattan_distance};
pub use world_position::WorldPosition;
