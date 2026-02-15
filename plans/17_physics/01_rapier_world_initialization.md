# Rapier World Initialization

## Problem

A voxel-based game engine with planetary-scale worlds needs a robust, deterministic physics simulation from the very first frame. Without a properly initialized physics pipeline, nothing can collide, fall, or interact physically. The engine requires a central physics world that integrates with the ECS `FixedUpdate` schedule at a stable 60 Hz tick rate, provides configurable gravity (which must be planet-relative, not an absolute downward vector), and exposes the full Rapier pipeline — broad phase, narrow phase, solver, CCD — through a clean abstraction. Because Nebula Engine uses 128-bit world coordinates but Rapier operates entirely in f32 local space, the physics world must be designed from the start as a local-space construct that never sees raw i128 positions. The `PhysicsWorld` wrapper must own all Rapier state and present a minimal, engine-friendly API that hides the dozen interacting Rapier structs (rigid body set, collider set, impulse joint set, multibody joint set, island manager, broad phase, narrow phase, integration parameters, physics pipeline, query pipeline, CCD solver) behind a single resource.

## Solution

### PhysicsWorld Resource

Create a `PhysicsWorld` struct that owns every piece of Rapier state required to run the simulation:

```rust
use rapier3d::prelude::*;

pub struct PhysicsWorld {
    pub gravity: Vector<f32>,
    pub integration_parameters: IntegrationParameters,
    pub physics_pipeline: PhysicsPipeline,
    pub island_manager: IslandManager,
    pub broad_phase: DefaultBroadPhase,
    pub narrow_phase: NarrowPhase,
    pub rigid_body_set: RigidBodySet,
    pub collider_set: ColliderSet,
    pub impulse_joint_set: ImpulseJointSet,
    pub multibody_joint_set: MultibodyJointSet,
    pub ccd_solver: CCDSolver,
    pub query_pipeline: QueryPipeline,
}
```

The constructor `PhysicsWorld::new()` initializes all fields with Rapier defaults and sets gravity to `Vector::new(0.0, -9.81, 0.0)`. Gravity here is expressed relative to the local physics frame — not in absolute world space. In planetary contexts a separate gravity system (see story 07) overrides this per-entity, but the default provides a sane starting point for surface gameplay.

### Integration Parameters

`IntegrationParameters` is configured with:

- `dt`: `1.0 / 60.0` (matching the `FixedUpdate` rate)
- `min_ccd_dt`: `1.0 / (60.0 * 100.0)` (CCD substep floor)
- `max_velocity_iterations`: `4`
- `max_stabilization_iterations`: `1`

These defaults balance accuracy and performance for a voxel game where most collisions are axis-aligned and simple.

### Stepping the Simulation

A `step` method advances the simulation by one fixed timestep:

```rust
impl PhysicsWorld {
    pub fn step(&mut self) {
        self.physics_pipeline.step(
            &self.gravity,
            &self.integration_parameters,
            &mut self.island_manager,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.rigid_body_set,
            &mut self.collider_set,
            &mut self.impulse_joint_set,
            &mut self.multibody_joint_set,
            &mut self.ccd_solver,
            None, // query filter callback
            &(), // event handler
            &(), // contact modification handler
        );
        self.query_pipeline.update(
            &self.rigid_body_set,
            &self.collider_set,
        );
    }
}
```

The `QueryPipeline` is rebuilt after each step so that raycasts and shape casts reflect the latest body positions.

### ECS Integration

Register `PhysicsWorld` as a `bevy_ecs` resource. A system `physics_step_system` runs in the `FixedUpdate` schedule at 60 Hz:

```rust
fn physics_step_system(mut physics: ResMut<PhysicsWorld>) {
    physics.step();
}
```

The `FixedUpdate` schedule accumulates real elapsed time and calls this system zero or more times per frame, ensuring deterministic behavior regardless of render frame rate. When the frame rate drops below 60 FPS, multiple physics steps execute per frame; when above, some frames skip the physics step entirely.

### Gravity Configuration

Gravity is stored as a `Vector<f32>` (Rapier's nalgebra-backed type) and can be changed at runtime. A helper method normalizes the gravity API:

```rust
impl PhysicsWorld {
    pub fn set_gravity(&mut self, x: f32, y: f32, z: f32) {
        self.gravity = Vector::new(x, y, z);
    }

    pub fn gravity(&self) -> (f32, f32, f32) {
        (self.gravity.x, self.gravity.y, self.gravity.z)
    }
}
```

On planet surfaces, the gravity direction is dynamically computed per-entity (pointing toward the planet center in local space) and applied as a per-body force rather than through the world gravity vector. The world gravity serves as a fallback default.

## Outcome

A `PhysicsWorld` resource exists and is inserted into the Bevy ECS world at startup. The `FixedUpdate` schedule steps the simulation at 60 Hz. `cargo test -p nebula-physics` passes all physics-world initialization and stepping tests. Other systems can query `Res<PhysicsWorld>` for raycasts or `ResMut<PhysicsWorld>` to add/remove bodies and colliders.

## Demo Integration

**Demo crate:** `nebula-demo`

The Rapier physics world is created and stepped each fixed tick. No visible change yet, but the physics simulation clock is running.

## Crates & Dependencies

- `rapier3d = "0.32"` — Physics engine with broad/narrow phase, solver, CCD, and query pipeline
- `bevy_ecs = "0.18"` — ECS framework providing `Resource`, `ResMut`, schedule stages, and `FixedUpdate`
- `glam = "0.32"` — Vector math used by the engine; conversions to/from Rapier's nalgebra types as needed
- `nalgebra = "0.33"` — Pulled in transitively by Rapier; used for `Vector<f32>` gravity type

## Unit Tests

- **`test_physics_world_initializes`** — Construct `PhysicsWorld::new()`, assert `rigid_body_set.len() == 0`, `collider_set.len() == 0`, and that the pipeline fields are valid (not panic on construction). Verifies that all Rapier structs initialize without error.

- **`test_gravity_default`** — Construct `PhysicsWorld::new()`, call `gravity()`, assert the returned tuple is `(0.0, -9.81, 0.0)`. Verifies the default gravity direction and magnitude.

- **`test_gravity_set_custom`** — Construct a world, call `set_gravity(0.0, -1.62, 0.0)` (lunar gravity), assert `gravity()` returns `(0.0, -1.62, 0.0)`. Verifies runtime gravity modification.

- **`test_step_advances_simulation`** — Create a world, insert a dynamic rigid body at position `(0, 10, 0)` with no collider beneath it, step the simulation 60 times. Assert the body's y-position is less than 10.0 (it fell due to gravity). Verifies that stepping actually advances physics state.

- **`test_empty_world_steps_without_error`** — Construct an empty `PhysicsWorld`, call `step()` 100 times. Assert no panic. Verifies that the pipeline gracefully handles an empty simulation (no bodies, no colliders).

- **`test_timestep_matches_fixed_update`** — Construct a world, assert `integration_parameters.dt` equals `1.0 / 60.0` within f32 epsilon. Verifies that the physics timestep is synchronized with the intended `FixedUpdate` rate.
