//! Voxel storage with palette compression, chunk data structures, and chunk lifecycle management.

pub mod bit_packed;
pub mod chunk;
pub mod registry;

pub use chunk::{CHUNK_SIZE, CHUNK_VOLUME, ChunkData};
pub use registry::{RegistryError, Transparency, VoxelTypeDef, VoxelTypeId, VoxelTypeRegistry};
