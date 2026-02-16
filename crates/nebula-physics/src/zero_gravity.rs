//! Zero-gravity physics: Newtonian inertia, thrust-based movement, and damping control.
//!
//! Entities far from gravity sources experience true zero-g: no damping, momentum
//! conservation, and thrust-only maneuvering. The [`SpaceObject`] component marks
//! entities that should have Newtonian behavior in space, while [`ThrustInput`]
//! provides ship-local force/torque application.

use bevy_ecs::prelude::*;
use glam::Vec3;
use rapier3d::prelude::*;

use crate::{LocalGravity, PhysicsWorld, RigidBodyHandle};

/// Helper to create a Rapier `Vector` from three f32 values.
fn vec3(x: f32, y: f32, z: f32) -> Vector {
    Vector::new(x, y, z)
}

/// Gravity magnitude (m/s²) below which an entity is considered in zero-gravity.
pub const ZERO_G_THRESHOLD: f32 = 0.01;

/// Default linear damping restored when an entity returns to a surface.
const DEFAULT_LINEAR_DAMPING: f32 = 0.5;

/// Default angular damping restored when an entity returns to a surface.
const DEFAULT_ANGULAR_DAMPING: f32 = 1.0;

/// Returns `true` if the given local gravity is below the zero-g threshold.
pub fn is_zero_gravity(gravity: &LocalGravity) -> bool {
    gravity.magnitude < ZERO_G_THRESHOLD
}

/// Marks an entity as a space-capable object with configurable damping behavior.
///
/// When `newtonian` is `true` and the entity is in zero-g, linear damping is set
/// to zero so velocity is maintained indefinitely. Angular damping can be overridden
/// per-object for gameplay feel.
#[derive(Component, Clone, Debug)]
pub struct SpaceObject {
    /// If true, this object has zero linear/angular damping in zero-g (pure Newtonian).
    pub newtonian: bool,
    /// Optional angular damping for gameplay feel (`0.0` = none, higher = heavier).
    /// `None` means pure Newtonian (zero angular damping in space).
    pub angular_damping_override: Option<f32>,
}

/// Thrust input for spaceship-style force/torque application.
///
/// Values in `linear` and `angular` are normalized inputs (typically -1..1)
/// that get scaled by `max_thrust` and `max_torque` respectively.
#[derive(Component, Clone, Debug)]
pub struct ThrustInput {
    /// Thrust direction in the ship's local frame (forward, up, right).
    pub linear: Vec3,
    /// Torque direction in the ship's local frame (pitch, yaw, roll).
    pub angular: Vec3,
    /// Maximum linear thrust force in Newtons.
    pub max_thrust: f32,
    /// Maximum torque in N·m.
    pub max_torque: f32,
}

impl Default for ThrustInput {
    fn default() -> Self {
        Self {
            linear: Vec3::ZERO,
            angular: Vec3::ZERO,
            max_thrust: 1000.0,
            max_torque: 100.0,
        }
    }
}

/// System that configures damping on space objects based on their gravity environment.
///
/// In zero-g with `newtonian = true`, linear damping is zero and angular damping
/// uses the override (or zero). On a surface, default damping values are restored.
pub fn configure_space_damping_system(
    mut physics: ResMut<PhysicsWorld>,
    query: Query<(&RigidBodyHandle, &SpaceObject, &LocalGravity)>,
) {
    for (handle, space_obj, gravity) in query.iter() {
        if let Some(body) = physics.rigid_body_set.get_mut(handle.0) {
            if is_zero_gravity(gravity) && space_obj.newtonian {
                body.set_linear_damping(0.0);
                body.set_angular_damping(space_obj.angular_damping_override.unwrap_or(0.0));
            } else {
                body.set_linear_damping(DEFAULT_LINEAR_DAMPING);
                body.set_angular_damping(DEFAULT_ANGULAR_DAMPING);
            }
        }
    }
}

/// System that applies ship-local thrust forces and torques to rigid bodies.
///
/// Transforms `ThrustInput` from the ship's local frame to world space using
/// the body's current rotation, then applies force and torque via Rapier.
pub fn apply_thrust_system(
    mut physics: ResMut<PhysicsWorld>,
    query: Query<(&RigidBodyHandle, &ThrustInput)>,
) {
    for (handle, thrust) in query.iter() {
        if let Some(body) = physics.rigid_body_set.get_mut(handle.0) {
            let rotation = *body.rotation();
            let world_thrust = rotation
                * vec3(
                    thrust.linear.x * thrust.max_thrust,
                    thrust.linear.y * thrust.max_thrust,
                    thrust.linear.z * thrust.max_thrust,
                );
            body.add_force(world_thrust, true);

            let world_torque = rotation
                * vec3(
                    thrust.angular.x * thrust.max_torque,
                    thrust.angular.y * thrust.max_torque,
                    thrust.angular.z * thrust.max_torque,
                );
            body.add_torque(world_torque, true);
        }
    }
}

/// Returns the angular velocity of a rigid body as a `Vec3`, or `None` if the handle is invalid.
pub fn get_angular_velocity(physics: &PhysicsWorld, handle: &RigidBodyHandle) -> Option<Vec3> {
    physics.rigid_body_set.get(handle.0).map(|body| {
        let av = body.angvel();
        Vec3::new(av.x, av.y, av.z)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a physics world with zero world gravity and step it.
    fn zero_g_world() -> PhysicsWorld {
        let mut world = PhysicsWorld::new();
        world.set_gravity(0.0, 0.0, 0.0);
        world
    }

    #[test]
    fn test_object_in_space_maintains_velocity() {
        let mut world = zero_g_world();

        let body = RigidBodyBuilder::dynamic()
            .translation(vec3(0.0, 0.0, 0.0))
            .linvel(vec3(10.0, 0.0, 0.0))
            .linear_damping(0.0)
            .angular_damping(0.0)
            .build();
        let handle = world.rigid_body_set.insert(body);
        let collider = ColliderBuilder::ball(0.5).build();
        world
            .collider_set
            .insert_with_parent(collider, handle, &mut world.rigid_body_set);

        for _ in 0..600 {
            world.step();
        }

        let pos = world.rigid_body_set[handle].translation();
        // 10 m/s * 10 s = 100 m
        assert!((pos.x - 100.0).abs() < 1.0, "Expected x≈100, got {}", pos.x);
        assert!(pos.y.abs() < 0.1, "Expected y≈0, got {}", pos.y);
        assert!(pos.z.abs() < 0.1, "Expected z≈0, got {}", pos.z);
    }

    #[test]
    fn test_no_acceleration_without_thrust() {
        let mut world = zero_g_world();

        let body = RigidBodyBuilder::dynamic()
            .translation(vec3(0.0, 0.0, 0.0))
            .linvel(vec3(0.0, 0.0, 0.0))
            .linear_damping(0.0)
            .build();
        let handle = world.rigid_body_set.insert(body);
        let collider = ColliderBuilder::ball(0.5).build();
        world
            .collider_set
            .insert_with_parent(collider, handle, &mut world.rigid_body_set);

        for _ in 0..120 {
            world.step();
        }

        let pos = world.rigid_body_set[handle].translation();
        assert!(pos.x.abs() < f32::EPSILON, "Expected x≈0, got {}", pos.x);
        assert!(pos.y.abs() < f32::EPSILON, "Expected y≈0, got {}", pos.y);
        assert!(pos.z.abs() < f32::EPSILON, "Expected z≈0, got {}", pos.z);
    }

    #[test]
    fn test_collision_in_zero_g_conserves_momentum() {
        let mut world = zero_g_world();

        let mass = 1.0_f32;

        // Body A moving right at 10 m/s
        let body_a = RigidBodyBuilder::dynamic()
            .translation(vec3(0.0, 0.0, 0.0))
            .linvel(vec3(10.0, 0.0, 0.0))
            .linear_damping(0.0)
            .build();
        let handle_a = world.rigid_body_set.insert(body_a);
        let col_a = ColliderBuilder::ball(0.5)
            .restitution(1.0)
            .density(mass / (4.0 / 3.0 * std::f32::consts::PI * 0.5_f32.powi(3)))
            .build();
        world
            .collider_set
            .insert_with_parent(col_a, handle_a, &mut world.rigid_body_set);

        // Body B stationary at x=3
        let body_b = RigidBodyBuilder::dynamic()
            .translation(vec3(3.0, 0.0, 0.0))
            .linvel(vec3(0.0, 0.0, 0.0))
            .linear_damping(0.0)
            .build();
        let handle_b = world.rigid_body_set.insert(body_b);
        let col_b = ColliderBuilder::ball(0.5)
            .restitution(1.0)
            .density(mass / (4.0 / 3.0 * std::f32::consts::PI * 0.5_f32.powi(3)))
            .build();
        world
            .collider_set
            .insert_with_parent(col_b, handle_b, &mut world.rigid_body_set);

        let mass_a = world.rigid_body_set[handle_a].mass();
        let mass_b = world.rigid_body_set[handle_b].mass();
        let vel_a_before = world.rigid_body_set[handle_a].linvel();
        let vel_b_before = world.rigid_body_set[handle_b].linvel();
        let momentum_before = mass_a * vel_a_before.x + mass_b * vel_b_before.x;

        // Step until collision and separation
        for _ in 0..300 {
            world.step();
        }

        let vel_a_after = world.rigid_body_set[handle_a].linvel();
        let vel_b_after = world.rigid_body_set[handle_b].linvel();
        let momentum_after = mass_a * vel_a_after.x + mass_b * vel_b_after.x;

        let tolerance = momentum_before.abs() * 0.01;
        assert!(
            (momentum_after - momentum_before).abs() < tolerance,
            "Momentum not conserved: before={}, after={}",
            momentum_before,
            momentum_after
        );
    }

    #[test]
    fn test_torque_causes_rotation() {
        let mut world = zero_g_world();

        let body = RigidBodyBuilder::dynamic()
            .translation(vec3(0.0, 0.0, 0.0))
            .angular_damping(0.0)
            .build();
        let handle = world.rigid_body_set.insert(body);
        let collider = ColliderBuilder::ball(0.5).build();
        world
            .collider_set
            .insert_with_parent(collider, handle, &mut world.rigid_body_set);

        // Apply torque impulse for one tick
        world.rigid_body_set[handle].apply_torque_impulse(vec3(0.0, 1.0, 0.0), true);
        world.step();

        let av_after_one = world.rigid_body_set[handle].angvel().y;
        assert!(
            av_after_one > 0.0,
            "Angular velocity should be > 0 after torque, got {}",
            av_after_one
        );

        // Step 60 more ticks with no torque
        for _ in 0..60 {
            world.step();
        }

        let av_after_many = world.rigid_body_set[handle].angvel().y;
        assert!(
            (av_after_many - av_after_one).abs() < 0.01,
            "Angular velocity should persist: initial={}, after={}",
            av_after_one,
            av_after_many
        );
    }

    #[test]
    fn test_angular_damping_slows_rotation() {
        let mut world = zero_g_world();

        let body = RigidBodyBuilder::dynamic()
            .translation(vec3(0.0, 0.0, 0.0))
            .angvel(vec3(0.0, 10.0, 0.0))
            .angular_damping(2.0)
            .build();
        let handle = world.rigid_body_set.insert(body);
        let collider = ColliderBuilder::ball(0.5).build();
        world
            .collider_set
            .insert_with_parent(collider, handle, &mut world.rigid_body_set);

        for _ in 0..120 {
            world.step();
        }

        let av = world.rigid_body_set[handle].angvel();
        let mag = (av.x * av.x + av.y * av.y + av.z * av.z).sqrt();
        assert!(
            mag < 5.0,
            "Angular velocity should be significantly damped, got {}",
            mag
        );
    }

    #[test]
    fn test_thrust_produces_acceleration() {
        let mut world = zero_g_world();

        let body = RigidBodyBuilder::dynamic()
            .translation(vec3(0.0, 0.0, 0.0))
            .linear_damping(0.0)
            .build();
        let handle = world.rigid_body_set.insert(body);
        let collider = ColliderBuilder::ball(0.5).build();
        world
            .collider_set
            .insert_with_parent(collider, handle, &mut world.rigid_body_set);

        // Apply thrust in +Z for 60 ticks using per-step impulses
        let thrust_force = 1000.0_f32;
        let dt = world.integration_parameters.dt;
        for _ in 0..60 {
            world.rigid_body_set[handle].apply_impulse(vec3(0.0, 0.0, thrust_force * dt), true);
            world.step();
        }

        let vel = world.rigid_body_set[handle].linvel();
        let pos = world.rigid_body_set[handle].translation();
        assert!(
            vel.z > 0.0,
            "Z velocity should be positive after thrust, got {}",
            vel.z
        );
        assert!(
            pos.z > 0.0,
            "Z position should be positive after thrust, got {}",
            pos.z
        );

        // Remove thrust, reset forces, step 60 more ticks — velocity maintained
        world.rigid_body_set[handle].reset_forces(true);
        let vel_before = vel.z;
        for _ in 0..60 {
            world.step();
        }

        let vel_after = world.rigid_body_set[handle].linvel().z;
        assert!(
            (vel_after - vel_before).abs() < 0.1,
            "Velocity should be maintained without thrust: before={}, after={}",
            vel_before,
            vel_after
        );
    }
}
