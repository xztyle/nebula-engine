//! Level-of-detail management: distance-based LOD selection, transition blending, and LOD quadtree.

mod selector;

pub use selector::{LodSelector, LodThresholds, chunk_distance_to_camera};
