//! Meshing algorithms: greedy meshing, ambient occlusion, LOD stitching, and mesh data structures.

pub mod ambient_occlusion;
pub mod async_mesh;
pub mod chunk_mesh;
pub mod displacement;
pub mod face_direction;
pub mod greedy;
pub mod invalidation;
pub mod lod_stitching;
pub mod neighborhood;
pub mod packed;
pub mod transition_seams;
pub mod vertex_format;
pub mod visibility;
pub mod visible_faces;

pub use ambient_occlusion::{compute_face_ao, should_flip_ao_diagonal, vertex_ao};
pub use chunk_mesh::{ChunkMesh, MeshVertex, QuadInfo};
pub use face_direction::{CornerDirection, EdgeDirection, FaceDirection};
pub use greedy::greedy_mesh;
pub use neighborhood::{
    ChunkBoundaryEdge, ChunkBoundarySlice, ChunkNeighborhood, extract_boundary_slice,
};
pub use packed::{ChunkVertex, PackedChunkMesh};
pub use vertex_format::{CHUNK_VERTEX_ATTRIBUTES, CHUNK_VERTEX_LAYOUT, chunk_vertex_buffer_layout};
pub use visibility::{compute_visible_faces, count_total_faces, count_visible_faces};
pub use visible_faces::VisibleFaces;

pub use async_mesh::{MeshingPipeline, MeshingResult, MeshingTask};
pub use displacement::{DisplacementBuffer, PlanetParams, displace_to_cubesphere, displace_vertex};
pub use invalidation::{ChunkMeshState, MeshInvalidator};
pub use lod_stitching::{
    LodContext, apply_lod_stitching, generate_transition_strip, snap_edge_vertices,
};
pub use transition_seams::{
    ChunkLodContext, NeighborLodRelation, apply_seam_fix, constrain_edge_vertices, generate_skirt,
};
