//! Camera controllers, player physics bridge, and player state management.

pub mod camera_transition;
pub mod first_person_camera;
pub mod floating_origin;
pub mod free_fly_camera;
pub mod spaceship_controller;
pub mod third_person_camera;

pub use camera_transition::{
    CameraSnapshot, CameraTransition, EasingFunction, camera_transition_system,
};
pub use first_person_camera::{
    FirstPersonCamera, first_person_look_system, first_person_move_system,
};
pub use floating_origin::{
    ActiveCamera, FloatingOrigin, build_local_position_schedule, recompute_local_positions_system,
    update_floating_origin_system,
};
pub use free_fly_camera::{
    DebugCameraOverlay, FreeFlyCam, free_fly_look_system, free_fly_move_system,
    free_fly_overlay_system, free_fly_speed_system, free_fly_toggle_system,
};
pub use spaceship_controller::{
    SpaceshipController, apply_velocity_system, spaceship_rotation_system, spaceship_thrust_system,
};
pub use third_person_camera::{
    ThirdPersonCamera, third_person_follow_system, third_person_orbit_system,
    third_person_zoom_system,
};
