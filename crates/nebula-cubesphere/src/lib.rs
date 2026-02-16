//! Cube-sphere geometry: cube-to-sphere projection, face quadtree subdivision, and UV mapping.

mod chunk_address;
mod corner;
mod cross_face;
mod cube_face;
mod face_coord;
mod inverse;
mod neighbor;
mod projection;
mod quadtree;

pub use chunk_address::ChunkAddress;
pub use corner::{
    CornerNeighbors, CubeCorner, FaceCorner, corner_chunk_on_face, corner_lod_valid,
    face_corner_to_cube_corner,
};
pub use cross_face::{FaceEdgeAdjacency, face_adjacency, transform_uv_across_edge};
pub use cube_face::CubeFace;
pub use face_coord::FaceCoord;
pub use inverse::{direction_to_face, direction_to_face_coord, sphere_to_face_coord_everitt};
pub use neighbor::{FaceDirection, LodNeighbor, SameFaceNeighbor};
pub use projection::{
    ProjectionMethod, cube_to_sphere_everitt, face_coord_to_cube_point, face_coord_to_sphere,
    face_coord_to_sphere_everitt, project,
};
pub use quadtree::{FaceQuadtree, QuadNode};
