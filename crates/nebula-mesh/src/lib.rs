//! Meshing algorithms: greedy meshing, ambient occlusion, LOD stitching, and mesh data structures.

pub mod face_direction;
pub mod neighborhood;
pub mod visibility;
pub mod visible_faces;

pub use face_direction::FaceDirection;
pub use neighborhood::ChunkNeighborhood;
pub use visibility::{compute_visible_faces, count_total_faces, count_visible_faces};
pub use visible_faces::VisibleFaces;
