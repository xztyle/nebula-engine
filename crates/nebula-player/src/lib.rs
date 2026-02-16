//! Camera controllers, player physics bridge, and player state management.

pub mod first_person_camera;

pub use first_person_camera::{
    FirstPersonCamera, first_person_look_system, first_person_move_system,
};
