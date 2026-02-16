//! Meshing algorithms: greedy meshing, ambient occlusion, LOD stitching, and mesh data structures.

pub mod chunk_mesh;
pub mod face_direction;
pub mod greedy;
pub mod neighborhood;
pub mod visibility;
pub mod visible_faces;

pub use chunk_mesh::{ChunkMesh, MeshVertex, QuadInfo};
pub use face_direction::{CornerDirection, EdgeDirection, FaceDirection};
pub use greedy::greedy_mesh;
pub use neighborhood::{
    ChunkBoundaryEdge, ChunkBoundarySlice, ChunkNeighborhood, extract_boundary_slice,
};
pub use visibility::{compute_visible_faces, count_total_faces, count_visible_faces};
pub use visible_faces::VisibleFaces;
