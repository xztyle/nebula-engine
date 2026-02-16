//! Physics integration: rigid bodies, collision shapes, raycasting, and physics world stepping.
//!
//! Wraps the Rapier 3D physics engine behind a single [`PhysicsWorld`] resource
//! that owns all simulation state and exposes a minimal, engine-friendly API.

use bevy_ecs::prelude::*;
use rapier3d::prelude::*;

/// Central physics simulation resource owning all Rapier state.
///
/// Insert into the Bevy ECS world at startup. Systems read via `Res<PhysicsWorld>`
/// for raycasts or mutate via `ResMut<PhysicsWorld>` to add/remove bodies.
#[derive(Resource)]
pub struct PhysicsWorld {
    /// World-space gravity vector (local physics frame).
    pub gravity: Vector,
    /// Timestep and solver configuration.
    pub integration_parameters: IntegrationParameters,
    /// The main simulation pipeline.
    pub physics_pipeline: PhysicsPipeline,
    /// Tracks sleeping/awake body islands.
    pub island_manager: IslandManager,
    /// Broad-phase collision detection (also provides query pipeline).
    pub broad_phase: BroadPhaseBvh,
    /// Narrow-phase collision detection (contact manifolds).
    pub narrow_phase: NarrowPhase,
    /// All rigid bodies in the simulation.
    pub rigid_body_set: RigidBodySet,
    /// All colliders in the simulation.
    pub collider_set: ColliderSet,
    /// Impulse-based joints (ball, revolute, prismatic, fixed).
    pub impulse_joint_set: ImpulseJointSet,
    /// Multibody joints (reduced-coordinate articulations).
    pub multibody_joint_set: MultibodyJointSet,
    /// Continuous collision detection solver.
    pub ccd_solver: CCDSolver,
}

impl PhysicsWorld {
    /// Creates a new physics world with default gravity `(0, -9.81, 0)` and
    /// a timestep of `1/60` seconds matching the `FixedUpdate` rate.
    pub fn new() -> Self {
        let integration_parameters = IntegrationParameters {
            dt: 1.0 / 60.0,
            ..Default::default()
        };

        Self {
            gravity: Vector::new(0.0, -9.81, 0.0),
            integration_parameters,
            physics_pipeline: PhysicsPipeline::new(),
            island_manager: IslandManager::new(),
            broad_phase: BroadPhaseBvh::new(),
            narrow_phase: NarrowPhase::new(),
            rigid_body_set: RigidBodySet::new(),
            collider_set: ColliderSet::new(),
            impulse_joint_set: ImpulseJointSet::new(),
            multibody_joint_set: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
        }
    }

    /// Advances the simulation by one fixed timestep.
    pub fn step(&mut self) {
        self.physics_pipeline.step(
            self.gravity,
            &self.integration_parameters,
            &mut self.island_manager,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.rigid_body_set,
            &mut self.collider_set,
            &mut self.impulse_joint_set,
            &mut self.multibody_joint_set,
            &mut self.ccd_solver,
            &(),
            &(),
        );
    }

    /// Sets the world gravity vector.
    pub fn set_gravity(&mut self, x: f32, y: f32, z: f32) {
        self.gravity = Vector::new(x, y, z);
    }

    /// Returns the current gravity as `(x, y, z)`.
    pub fn gravity(&self) -> (f32, f32, f32) {
        (self.gravity.x, self.gravity.y, self.gravity.z)
    }
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new()
    }
}

/// ECS system that steps the physics simulation once per invocation.
///
/// Intended for the `FixedUpdate` schedule at 60 Hz.
pub fn physics_step_system(mut physics: ResMut<PhysicsWorld>) {
    physics.step();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_physics_world_initializes() {
        let world = PhysicsWorld::new();
        assert_eq!(world.rigid_body_set.len(), 0);
        assert_eq!(world.collider_set.len(), 0);
    }

    #[test]
    fn test_gravity_default() {
        let world = PhysicsWorld::new();
        let g = world.gravity();
        assert_eq!(g, (0.0, -9.81, 0.0));
    }

    #[test]
    fn test_gravity_set_custom() {
        let mut world = PhysicsWorld::new();
        world.set_gravity(0.0, -1.62, 0.0);
        assert_eq!(world.gravity(), (0.0, -1.62, 0.0));
    }

    #[test]
    fn test_step_advances_simulation() {
        let mut world = PhysicsWorld::new();
        let body = RigidBodyBuilder::dynamic()
            .translation(Vector::new(0.0, 10.0, 0.0))
            .build();
        let handle = world.rigid_body_set.insert(body);
        let collider = ColliderBuilder::ball(0.5).build();
        world
            .collider_set
            .insert_with_parent(collider, handle, &mut world.rigid_body_set);

        for _ in 0..60 {
            world.step();
        }

        let pos = world.rigid_body_set[handle].translation();
        assert!(pos.y < 10.0, "Body should have fallen: y={}", pos.y);
    }

    #[test]
    fn test_empty_world_steps_without_error() {
        let mut world = PhysicsWorld::new();
        for _ in 0..100 {
            world.step();
        }
    }

    #[test]
    fn test_timestep_matches_fixed_update() {
        let world = PhysicsWorld::new();
        let expected = 1.0_f32 / 60.0;
        assert!(
            (world.integration_parameters.dt - expected).abs() < f32::EPSILON,
            "dt={} expected={}",
            world.integration_parameters.dt,
            expected
        );
    }
}
