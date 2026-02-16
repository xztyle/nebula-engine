//! Voxel storage with palette compression, chunk data structures, and chunk lifecycle management.

pub mod registry;

pub use registry::{RegistryError, Transparency, VoxelTypeDef, VoxelTypeId, VoxelTypeRegistry};
