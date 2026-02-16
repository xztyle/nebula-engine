//! Camera controllers, player physics bridge, and player state management.

pub mod first_person_camera;
pub mod spaceship_controller;
pub mod third_person_camera;

pub use first_person_camera::{
    FirstPersonCamera, first_person_look_system, first_person_move_system,
};
pub use spaceship_controller::{
    SpaceshipController, apply_velocity_system, spaceship_rotation_system, spaceship_thrust_system,
};
pub use third_person_camera::{
    ThirdPersonCamera, third_person_follow_system, third_person_orbit_system,
    third_person_zoom_system,
};
