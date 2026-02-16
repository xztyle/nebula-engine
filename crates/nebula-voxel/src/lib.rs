//! Voxel storage with palette compression, chunk data structures, and chunk lifecycle management.

pub mod bit_packed;
pub mod chunk;
pub mod chunk_api;
pub mod chunk_loading;
pub mod chunk_manager;
pub mod chunk_serial;
pub mod registry;
pub mod rle;

pub use chunk::{CHUNK_SIZE, CHUNK_VOLUME, ChunkData};
pub use chunk_api::{Chunk, MESH_DIRTY, NETWORK_DIRTY, SAVE_DIRTY};
pub use chunk_loading::{ChunkLoadConfig, ChunkLoadQueue, ChunkLoadTickResult, ChunkLoader};
pub use chunk_manager::{ChunkAddress, ChunkManager};
pub use chunk_serial::{ChunkSerError, SerializeStats};
pub use registry::{RegistryError, Transparency, VoxelTypeDef, VoxelTypeId, VoxelTypeRegistry};
