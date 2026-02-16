//! Cube-sphere geometry: cube-to-sphere projection, face quadtree subdivision, and UV mapping.

mod cube_face;
mod face_coord;
mod projection;

pub use cube_face::CubeFace;
pub use face_coord::FaceCoord;
pub use projection::{
    ProjectionMethod, cube_to_sphere_everitt, face_coord_to_cube_point, face_coord_to_sphere,
    face_coord_to_sphere_everitt, project,
};
