//! Level-of-detail management: distance-based LOD selection, transition blending, and LOD quadtree.

mod face_quadtree_lod;
mod selector;

pub use face_quadtree_lod::{FaceQuadtreeLod, LodAction, LodChunkDescriptor};
pub use selector::{LodSelector, LodThresholds, chunk_distance_to_camera};
