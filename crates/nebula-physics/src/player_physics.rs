//! Player character physics: kinematic character controller with capsule collider.
//!
//! Provides [`PlayerPhysics`] for game-feel movement: walking, jumping, gravity,
//! stair stepping, slope limits, and ground detection via Rapier 0.32's
//! [`KinematicCharacterController`].

use rapier3d::control::{CharacterAutostep, CharacterLength, KinematicCharacterController};
use rapier3d::prelude::*;

use crate::PhysicsWorld;

/// Player character physics state: kinematic body + capsule collider + controller.
///
/// The player uses a kinematic position-based rigid body driven by input, not forces.
/// Rapier's [`KinematicCharacterController`] resolves collisions, handles stair
/// stepping, and detects ground contact.
pub struct PlayerPhysics {
    /// Handle to the kinematic rigid body in the physics world.
    pub body_handle: rapier3d::dynamics::RigidBodyHandle,
    /// Handle to the capsule collider attached to the body.
    pub collider_handle: rapier3d::geometry::ColliderHandle,
    /// Rapier's built-in character controller with tuned parameters.
    pub controller: KinematicCharacterController,
    /// Whether the player is currently standing on ground.
    pub grounded: bool,
    /// Current vertical velocity in m/s (positive = up).
    pub vertical_velocity: f32,
}

/// Default walk speed in m/s.
pub const WALK_SPEED: f32 = 5.0;
/// Jump impulse in m/s (instant upward velocity).
pub const JUMP_IMPULSE: f32 = 7.0;
/// Capsule half-height of the cylindrical segment (meters).
const CAPSULE_HALF_HEIGHT: f32 = 0.6;
/// Capsule radius (meters).
const CAPSULE_RADIUS: f32 = 0.3;

/// Spawns a player physics entity: kinematic body + capsule collider + controller.
///
/// The capsule is 1.8m tall (2×0.6 half-height + 2×0.3 radius) with 0.3m radius,
/// fitting through 1-block corridors and under 2-block doorways.
pub fn spawn_player_physics(physics: &mut PhysicsWorld, local_pos: glam::Vec3) -> PlayerPhysics {
    let body = RigidBodyBuilder::kinematic_position_based()
        .translation(Vector::new(local_pos.x, local_pos.y, local_pos.z))
        .build();
    let body_handle = physics.rigid_body_set.insert(body);

    // Capsule: total height = 2*0.6 + 2*0.3 = 1.8m
    let collider = ColliderBuilder::capsule_y(CAPSULE_HALF_HEIGHT, CAPSULE_RADIUS)
        .friction(0.0)
        .build();
    let collider_handle =
        physics
            .collider_set
            .insert_with_parent(collider, body_handle, &mut physics.rigid_body_set);

    let controller = KinematicCharacterController {
        max_slope_climb_angle: std::f32::consts::FRAC_PI_4, // 45°
        min_slope_slide_angle: std::f32::consts::FRAC_PI_4,
        autostep: Some(CharacterAutostep {
            max_height: CharacterLength::Absolute(0.5),
            min_width: CharacterLength::Absolute(0.3),
            include_dynamic_bodies: false,
        }),
        snap_to_ground: Some(CharacterLength::Absolute(0.2)),
        offset: CharacterLength::Absolute(0.01),
        ..Default::default()
    };

    PlayerPhysics {
        body_handle,
        collider_handle,
        controller,
        grounded: false,
        vertical_velocity: 0.0,
    }
}

/// Applies one tick of player movement: horizontal walk + vertical gravity/jump.
///
/// `horizontal` is the desired XZ movement direction (normalized or zero),
/// `jump` is true if the jump action was triggered this tick,
/// `dt` is the fixed timestep in seconds.
///
/// Internally calls `KinematicCharacterController::move_shape` to resolve
/// collisions, then updates the body position and grounded state.
pub fn player_movement_step(
    player: &mut PlayerPhysics,
    physics: &mut PhysicsWorld,
    horizontal: glam::Vec3,
    jump: bool,
    dt: f32,
) {
    let gravity_y = physics.gravity.y; // typically -9.81

    // Vertical logic
    if player.grounded {
        player.vertical_velocity = 0.0;
        if jump {
            player.vertical_velocity = JUMP_IMPULSE;
        }
    } else {
        player.vertical_velocity += gravity_y * dt;
    }

    let walk = horizontal * WALK_SPEED;
    let desired = Vector::new(walk.x * dt, player.vertical_velocity * dt, walk.z * dt);

    // Build query pipeline from broad phase
    let filter = QueryFilter::new().exclude_rigid_body(player.body_handle);
    let query_pipeline = physics.broad_phase.as_query_pipeline(
        physics.narrow_phase.query_dispatcher(),
        &physics.rigid_body_set,
        &physics.collider_set,
        filter,
    );

    let character_shape = Capsule::new_y(CAPSULE_HALF_HEIGHT, CAPSULE_RADIUS);
    let body_pos = physics.rigid_body_set[player.body_handle].position();

    let corrected = player.controller.move_shape(
        dt,
        &query_pipeline,
        &character_shape,
        body_pos,
        desired,
        |_| {},
    );

    // Apply corrected movement
    let body = &mut physics.rigid_body_set[player.body_handle];
    let new_translation = body.translation() + corrected.translation;
    body.set_next_kinematic_translation(new_translation);

    player.grounded = corrected.grounded;
}

/// Performs a downward raycast to detect ground beneath the player.
///
/// Returns `true` if a surface is found within `max_distance` below the body origin.
/// This supplements the controller's built-in `grounded` flag.
pub fn ground_raycast(physics: &PhysicsWorld, player: &PlayerPhysics) -> bool {
    let body = &physics.rigid_body_set[player.body_handle];
    let origin = body.translation();
    let ray = Ray::new(origin, Vector::new(0.0, -1.0, 0.0));
    let max_distance = CAPSULE_HALF_HEIGHT + CAPSULE_RADIUS + 0.1;

    let filter = QueryFilter::new().exclude_rigid_body(player.body_handle);
    let query_pipeline = physics.broad_phase.as_query_pipeline(
        physics.narrow_phase.query_dispatcher(),
        &physics.rigid_body_set,
        &physics.collider_set,
        filter,
    );

    query_pipeline.cast_ray(&ray, max_distance, true).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PhysicsWorld;

    /// Helper: create a flat floor collider at y=0 (a thin cuboid spanning 100x1x100).
    fn add_floor(physics: &mut PhysicsWorld) -> rapier3d::geometry::ColliderHandle {
        let floor_body = RigidBodyBuilder::fixed()
            .translation(Vector::new(0.0, -0.5, 0.0))
            .build();
        let floor_handle = physics.rigid_body_set.insert(floor_body);
        let floor_collider = ColliderBuilder::cuboid(50.0, 0.5, 50.0).build();
        physics.collider_set.insert_with_parent(
            floor_collider,
            floor_handle,
            &mut physics.rigid_body_set,
        )
    }

    /// Helper: step physics + player movement for N ticks.
    fn step_n(
        player: &mut PlayerPhysics,
        physics: &mut PhysicsWorld,
        n: usize,
        horizontal: glam::Vec3,
        jump: bool,
    ) {
        let dt = 1.0 / 60.0;
        for i in 0..n {
            // Only jump on first tick
            let j = jump && i == 0;
            physics.step();
            player_movement_step(player, physics, horizontal, j, dt);
            physics.step();
        }
    }

    #[test]
    fn test_player_stands_on_solid_ground() {
        let mut physics = PhysicsWorld::new();
        add_floor(&mut physics);

        // Spawn player 2m above floor
        let mut player = spawn_player_physics(&mut physics, glam::Vec3::new(0.0, 2.0, 0.0));

        step_n(&mut player, &mut physics, 120, glam::Vec3::ZERO, false);

        let y = physics.rigid_body_set[player.body_handle].translation().y;
        // Capsule center should be at ~0.9m (half-height above floor)
        assert!(
            (y - 0.9).abs() < 0.3,
            "Player should stabilize near y=0.9, got y={y}"
        );
        assert!(player.grounded, "Player should be grounded on flat floor");
    }

    #[test]
    fn test_player_cannot_walk_through_walls() {
        let mut physics = PhysicsWorld::new();
        add_floor(&mut physics);

        // Wall at x=5, spanning y=0..3, z=-50..50
        let wall_body = RigidBodyBuilder::fixed()
            .translation(Vector::new(5.0, 1.5, 0.0))
            .build();
        let wall_handle = physics.rigid_body_set.insert(wall_body);
        let wall_collider = ColliderBuilder::cuboid(0.5, 1.5, 50.0).build();
        physics.collider_set.insert_with_parent(
            wall_collider,
            wall_handle,
            &mut physics.rigid_body_set,
        );

        // Spawn player at x=2, on the floor
        let mut player = spawn_player_physics(&mut physics, glam::Vec3::new(2.0, 0.9, 0.0));

        // Let player settle, then walk toward wall (+X)
        step_n(&mut player, &mut physics, 30, glam::Vec3::ZERO, false);
        step_n(
            &mut player,
            &mut physics,
            60,
            glam::Vec3::new(1.0, 0.0, 0.0),
            false,
        );

        let x = physics.rigid_body_set[player.body_handle].translation().x;
        // Wall face is at x=4.5, player capsule radius is 0.3, skin is 0.01
        assert!(
            x < 4.5,
            "Player should not cross wall plane at x=4.5, got x={x}"
        );
    }

    #[test]
    fn test_jump_applies_upward_velocity() {
        let mut physics = PhysicsWorld::new();
        add_floor(&mut physics);

        let mut player = spawn_player_physics(&mut physics, glam::Vec3::new(0.0, 0.9, 0.0));

        // Settle on ground
        step_n(&mut player, &mut physics, 60, glam::Vec3::ZERO, false);
        assert!(player.grounded, "Player should be grounded before jump");

        let rest_y = physics.rigid_body_set[player.body_handle].translation().y;

        // Jump
        step_n(&mut player, &mut physics, 1, glam::Vec3::ZERO, true);
        assert!(
            player.vertical_velocity > 0.0,
            "Vertical velocity should be positive after jump"
        );

        // Step forward to reach peak
        step_n(&mut player, &mut physics, 30, glam::Vec3::ZERO, false);
        let peak_y = physics.rigid_body_set[player.body_handle].translation().y;
        assert!(
            peak_y > rest_y + 0.1,
            "Player should rise above rest position: peak_y={peak_y}, rest_y={rest_y}"
        );

        // Step more to return to ground
        step_n(&mut player, &mut physics, 90, glam::Vec3::ZERO, false);
        let final_y = physics.rigid_body_set[player.body_handle].translation().y;
        assert!(
            (final_y - rest_y).abs() < 0.5,
            "Player should return near rest: final_y={final_y}, rest_y={rest_y}"
        );
    }

    #[test]
    fn test_ground_detection_on_flat_surface() {
        let mut physics = PhysicsWorld::new();
        let floor_collider_handle = add_floor(&mut physics);

        let mut player = spawn_player_physics(&mut physics, glam::Vec3::new(0.0, 0.9, 0.0));
        step_n(&mut player, &mut physics, 60, glam::Vec3::ZERO, false);
        assert!(player.grounded, "Player should be grounded on floor");

        // Remove the floor
        let parent = physics
            .collider_set
            .get(floor_collider_handle)
            .and_then(|c| c.parent());
        if let Some(parent_handle) = parent {
            physics.rigid_body_set.remove(
                parent_handle,
                &mut physics.island_manager,
                &mut physics.collider_set,
                &mut physics.impulse_joint_set,
                &mut physics.multibody_joint_set,
                true,
            );
        }

        step_n(&mut player, &mut physics, 10, glam::Vec3::ZERO, false);
        assert!(
            !player.grounded,
            "Player should not be grounded after floor removal"
        );
    }

    #[test]
    fn test_stair_stepping_climbs_small_steps() {
        let mut physics = PhysicsWorld::new();

        // Lower floor: y=-0.5..0 for x < 5
        let lower_body = RigidBodyBuilder::fixed()
            .translation(Vector::new(0.0, -0.5, 0.0))
            .build();
        let lower_handle = physics.rigid_body_set.insert(lower_body);
        physics.collider_set.insert_with_parent(
            ColliderBuilder::cuboid(50.0, 0.5, 50.0).build(),
            lower_handle,
            &mut physics.rigid_body_set,
        );

        // Step: a block at y=0..0.5 for x >= 4.5
        let step_body = RigidBodyBuilder::fixed()
            .translation(Vector::new(7.0, 0.25, 0.0))
            .build();
        let step_handle = physics.rigid_body_set.insert(step_body);
        physics.collider_set.insert_with_parent(
            ColliderBuilder::cuboid(3.0, 0.25, 50.0).build(),
            step_handle,
            &mut physics.rigid_body_set,
        );

        // Spawn player on lower floor, walk toward step
        let mut player = spawn_player_physics(&mut physics, glam::Vec3::new(0.0, 0.9, 0.0));
        step_n(&mut player, &mut physics, 30, glam::Vec3::ZERO, false);

        let start_y = physics.rigid_body_set[player.body_handle].translation().y;

        // Walk toward and over the step
        step_n(
            &mut player,
            &mut physics,
            120,
            glam::Vec3::new(1.0, 0.0, 0.0),
            false,
        );

        let end_y = physics.rigid_body_set[player.body_handle].translation().y;
        assert!(
            end_y > start_y + 0.3,
            "Player should climb the step: start_y={start_y}, end_y={end_y}"
        );
    }

    #[test]
    fn test_slope_slide_above_max_angle() {
        let mut physics = PhysicsWorld::new();

        // Create a steep slope (60°) using a rotated cuboid.
        // We approximate with a wedge: a thin tilted surface.
        // Simpler approach: place a floor, then a steep ramp as a rotated body.
        let angle_rad = 60.0_f32.to_radians();
        let ramp_body = RigidBodyBuilder::fixed()
            .translation(Vector::new(5.0, 2.0, 0.0))
            .rotation(Vector::new(0.0, 0.0, angle_rad))
            .build();
        let ramp_handle = physics.rigid_body_set.insert(ramp_body);
        physics.collider_set.insert_with_parent(
            ColliderBuilder::cuboid(5.0, 0.1, 50.0).build(),
            ramp_handle,
            &mut physics.rigid_body_set,
        );

        // Flat floor below so player doesn't fall forever
        let floor_body = RigidBodyBuilder::fixed()
            .translation(Vector::new(0.0, -0.5, 0.0))
            .build();
        let floor_handle = physics.rigid_body_set.insert(floor_body);
        physics.collider_set.insert_with_parent(
            ColliderBuilder::cuboid(50.0, 0.5, 50.0).build(),
            floor_handle,
            &mut physics.rigid_body_set,
        );

        // Spawn player at base of ramp
        let mut player = spawn_player_physics(&mut physics, glam::Vec3::new(2.0, 0.9, 0.0));
        step_n(&mut player, &mut physics, 30, glam::Vec3::ZERO, false);

        let start_y = physics.rigid_body_set[player.body_handle].translation().y;

        // Try to walk up the steep slope
        step_n(
            &mut player,
            &mut physics,
            60,
            glam::Vec3::new(1.0, 0.0, 0.0),
            false,
        );

        let end_y = physics.rigid_body_set[player.body_handle].translation().y;
        // Player should NOT have climbed significantly (slope > 45° limit)
        assert!(
            end_y < start_y + 1.0,
            "Player should not climb steep slope: start_y={start_y}, end_y={end_y}"
        );
    }
}
