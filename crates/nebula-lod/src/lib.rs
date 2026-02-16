//! Level-of-detail management: distance-based LOD selection, transition blending, and LOD quadtree.

mod face_quadtree_lod;
mod memory_budget;
mod priority_queue;
mod selector;

pub use face_quadtree_lod::{FaceQuadtreeLod, LodAction, LodChunkDescriptor};
pub use memory_budget::{
    ChunkMemoryUsage, MemoryBudgetConfig, MemoryBudgetTracker, select_evictions,
};
pub use priority_queue::{ChunkPriorityFactors, LodPriorityQueue, compute_priority};
pub use selector::{LodSelector, LodThresholds, chunk_distance_to_camera};
