# System Ordering

## Problem

Within each execution stage, the engine may have dozens of systems. While bevy_ecs runs non-conflicting systems in parallel by default, some systems have logical dependencies that the borrow checker cannot infer. For example: the input polling system must finish writing `InputState` before the player movement system reads it. The physics step must finish updating `WorldPos` before the collision response system reads it. The frustum culling system must finish before the draw batching system reads the visibility results. If these orderings are not explicitly declared, bevy_ecs may schedule them in any order (or concurrently), producing nondeterministic behavior, stale reads, or subtle frame-order bugs that only manifest under specific timing conditions.

bevy_ecs provides `.before()`, `.after()`, and `.in_set()` APIs for declaring ordering constraints. The engine needs a systematic approach to these constraints: named system sets that group related systems, explicit inter-set orderings, and validation that the resulting dependency graph is acyclic.

## Solution

### System Set Definitions

Define `SystemSet` enums for each major subsystem. These sets group related systems and serve as targets for ordering constraints:

```rust
use bevy_ecs::prelude::*;

/// Sets for systems in the PreUpdate stage.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum PreUpdateSet {
    /// Advance time counters, poll OS events.
    Time,
    /// Process raw input into action-based InputState.
    Input,
}

/// Sets for systems in the FixedUpdate stage.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum FixedUpdateSet {
    /// Read input and compute movement intents.
    InputProcessing,
    /// Apply forces (gravity, player movement, explosions).
    ForceApplication,
    /// Step the physics engine (broadphase, narrowphase, solver).
    PhysicsStep,
    /// Resolve physics results back into WorldPos/Velocity.
    PhysicsSync,
}

/// Sets for systems in the Update stage.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum UpdateSet {
    /// AI decision-making, pathfinding, behavior trees.
    AI,
    /// Gameplay logic: health, inventory, triggers, scripting.
    Gameplay,
    /// Animation state machine updates.
    Animation,
}

/// Sets for systems in the PostUpdate stage.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum PostUpdateSet {
    /// Convert WorldPos to LocalPos relative to camera.
    TransformPropagation,
    /// Update spatial acceleration structures (octree, BVH).
    SpatialIndexUpdate,
}

/// Sets for systems in the PreRender stage.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum PreRenderSet {
    /// Frustum culling and visibility determination.
    Culling,
    /// Batch draw calls by material and mesh.
    Batching,
    /// Upload vertex/index/uniform buffers to GPU.
    BufferUpload,
}

/// Sets for systems in the Render stage.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum RenderSet {
    /// Acquire the swapchain surface texture.
    SurfaceAcquire,
    /// Execute draw commands.
    Draw,
    /// Present the frame.
    Present,
}
```

### Ordering Constraints

Register the ordering constraints when building each stage's schedule:

```rust
pub fn configure_preupdate_ordering(schedule: &mut Schedule) {
    schedule.configure_sets(
        PreUpdateSet::Time.before(PreUpdateSet::Input),
    );
}

pub fn configure_fixedupdate_ordering(schedule: &mut Schedule) {
    schedule.configure_sets((
        FixedUpdateSet::InputProcessing
            .before(FixedUpdateSet::ForceApplication),
        FixedUpdateSet::ForceApplication
            .before(FixedUpdateSet::PhysicsStep),
        FixedUpdateSet::PhysicsStep
            .before(FixedUpdateSet::PhysicsSync),
    ));
}

pub fn configure_update_ordering(schedule: &mut Schedule) {
    schedule.configure_sets((
        UpdateSet::AI.before(UpdateSet::Gameplay),
        UpdateSet::Gameplay.before(UpdateSet::Animation),
    ));
}

pub fn configure_postupdate_ordering(schedule: &mut Schedule) {
    schedule.configure_sets(
        PostUpdateSet::TransformPropagation
            .before(PostUpdateSet::SpatialIndexUpdate),
    );
}

pub fn configure_prerender_ordering(schedule: &mut Schedule) {
    schedule.configure_sets((
        PreRenderSet::Culling.before(PreRenderSet::Batching),
        PreRenderSet::Batching.before(PreRenderSet::BufferUpload),
    ));
}

pub fn configure_render_ordering(schedule: &mut Schedule) {
    schedule.configure_sets((
        RenderSet::SurfaceAcquire.before(RenderSet::Draw),
        RenderSet::Draw.before(RenderSet::Present),
    ));
}
```

### Registering Systems into Sets

Systems are assigned to sets when they are added to a schedule:

```rust
// Example: registering the input polling system
schedule.add_systems(
    poll_input_system.in_set(PreUpdateSet::Input),
);

// Example: registering physics systems with ordering
schedule.add_systems((
    apply_gravity_system.in_set(FixedUpdateSet::ForceApplication),
    apply_player_movement_system.in_set(FixedUpdateSet::ForceApplication),
    step_physics_world_system.in_set(FixedUpdateSet::PhysicsStep),
    sync_physics_results_system.in_set(FixedUpdateSet::PhysicsSync),
));
```

Systems within the same set that do not have explicit `.before()` / `.after()` constraints between them are free to run in parallel if their data access does not conflict. For example, `apply_gravity_system` and `apply_player_movement_system` are both in `ForceApplication` and can run concurrently because they write to different entities' `Velocity` components.

### Fine-Grained Ordering

For cases where ordering is needed within a set, use direct `.before()` / `.after()` on individual systems:

```rust
schedule.add_systems((
    compute_damage_system
        .in_set(UpdateSet::Gameplay)
        .before(apply_damage_system),
    apply_damage_system
        .in_set(UpdateSet::Gameplay)
        .before(check_death_system),
    check_death_system
        .in_set(UpdateSet::Gameplay),
));
```

### Cycle Detection

bevy_ecs validates the system dependency graph when the schedule is first built (on the first call to `schedule.run()`). If a cycle exists — A before B, B before C, C before A — bevy_ecs panics with a diagnostic error message listing the cycle. The engine does not need custom cycle detection; it relies on bevy_ecs's built-in validation.

To surface cycle errors early (at startup rather than mid-game), call `schedule.initialize(&mut world)` during engine initialization:

```rust
pub fn validate_schedules(schedules: &mut EngineSchedules, world: &mut World) {
    // Forces schedule graph validation for all stages.
    // Panics with a descriptive error if any cycle is detected.
    for (_label, schedule) in &mut schedules.schedules {
        schedule.initialize(world);
    }
}
```

### Independent Systems Run Unordered

Systems that are not assigned to any set and have no explicit ordering constraints run in whatever order the parallel executor chooses. This is intentional: the default is maximum parallelism. Only add ordering constraints when there is a logical dependency. Over-constraining the schedule reduces parallelism and hurts performance.

### Visualization (Debug)

In debug builds, the system graph can be dumped for inspection:

```rust
#[cfg(debug_assertions)]
pub fn print_schedule_graph(schedule: &Schedule) {
    // bevy_ecs provides schedule graph introspection that can be
    // used to generate DOT format output for visualization.
    // This is invaluable for debugging ordering issues.
    println!("{:#?}", schedule);
}
```

## Outcome

After this story is complete:

- Every major subsystem has a named `SystemSet` for grouping related systems
- Inter-set ordering constraints ensure data flows correctly: input before gameplay, gameplay before physics, physics before transform update, transforms before rendering
- Systems within the same set run in parallel by default unless explicitly ordered
- Cycle detection is performed at startup, preventing invalid ordering configurations from reaching gameplay
- Fine-grained `.before()` / `.after()` constraints handle intra-set dependencies
- The system graph can be inspected in debug builds for troubleshooting

## Demo Integration

**Demo crate:** `nebula-demo`

The terrain generation system runs before meshing, which runs before rendering. The console shows the dependency chain each frame.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.15` | `SystemSet` derive, `.before()`, `.after()`, `.in_set()`, schedule validation |

Rust edition 2024. No additional dependencies. System ordering is a built-in feature of bevy_ecs's schedule infrastructure.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::prelude::*;

    #[derive(Resource, Default)]
    struct ExecutionOrder(Vec<&'static str>);

    #[test]
    fn test_before_constraint_respected() {
        let mut world = World::new();
        world.insert_resource(ExecutionOrder::default());

        let mut schedule = Schedule::default();
        schedule.configure_sets(
            FixedUpdateSet::InputProcessing
                .before(FixedUpdateSet::PhysicsStep),
        );

        schedule.add_systems((
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("physics");
            })
            .in_set(FixedUpdateSet::PhysicsStep),
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("input");
            })
            .in_set(FixedUpdateSet::InputProcessing),
        ));

        schedule.run(&mut world);

        let order = world.resource::<ExecutionOrder>();
        let input_idx = order.0.iter().position(|s| *s == "input").unwrap();
        let physics_idx = order.0.iter().position(|s| *s == "physics").unwrap();
        assert!(
            input_idx < physics_idx,
            "InputProcessing ({}) must run before PhysicsStep ({})",
            input_idx,
            physics_idx
        );
    }

    #[test]
    fn test_after_constraint_respected() {
        let mut world = World::new();
        world.insert_resource(ExecutionOrder::default());

        let mut schedule = Schedule::default();
        schedule.configure_sets(
            PostUpdateSet::SpatialIndexUpdate
                .after(PostUpdateSet::TransformPropagation),
        );

        schedule.add_systems((
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("spatial");
            })
            .in_set(PostUpdateSet::SpatialIndexUpdate),
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("transform");
            })
            .in_set(PostUpdateSet::TransformPropagation),
        ));

        schedule.run(&mut world);

        let order = world.resource::<ExecutionOrder>();
        let transform_idx = order.0.iter().position(|s| *s == "transform").unwrap();
        let spatial_idx = order.0.iter().position(|s| *s == "spatial").unwrap();
        assert!(transform_idx < spatial_idx);
    }

    #[test]
    fn test_system_sets_group_correctly() {
        let mut world = World::new();
        world.insert_resource(ExecutionOrder::default());

        let mut schedule = Schedule::default();
        configure_prerender_ordering(&mut schedule);

        schedule.add_systems((
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("cull");
            })
            .in_set(PreRenderSet::Culling),
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("batch");
            })
            .in_set(PreRenderSet::Batching),
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("upload");
            })
            .in_set(PreRenderSet::BufferUpload),
        ));

        schedule.run(&mut world);

        let order = world.resource::<ExecutionOrder>();
        let cull_idx = order.0.iter().position(|s| *s == "cull").unwrap();
        let batch_idx = order.0.iter().position(|s| *s == "batch").unwrap();
        let upload_idx = order.0.iter().position(|s| *s == "upload").unwrap();
        assert!(cull_idx < batch_idx, "culling before batching");
        assert!(batch_idx < upload_idx, "batching before upload");
    }

    #[test]
    fn test_full_chain_input_to_physics_to_sync() {
        let mut world = World::new();
        world.insert_resource(ExecutionOrder::default());

        let mut schedule = Schedule::default();
        configure_fixedupdate_ordering(&mut schedule);

        schedule.add_systems((
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("input");
            })
            .in_set(FixedUpdateSet::InputProcessing),
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("forces");
            })
            .in_set(FixedUpdateSet::ForceApplication),
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("physics");
            })
            .in_set(FixedUpdateSet::PhysicsStep),
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("sync");
            })
            .in_set(FixedUpdateSet::PhysicsSync),
        ));

        schedule.run(&mut world);

        let order = world.resource::<ExecutionOrder>();
        assert_eq!(order.0, vec!["input", "forces", "physics", "sync"]);
    }

    #[test]
    fn test_independent_systems_run_unordered() {
        // Two systems in the same set with no explicit ordering between
        // them should both run, though order is nondeterministic.
        let mut world = World::new();

        #[derive(Resource, Default)]
        struct CountA(u32);
        #[derive(Resource, Default)]
        struct CountB(u32);
        world.insert_resource(CountA::default());
        world.insert_resource(CountB::default());

        let mut schedule = Schedule::default();
        schedule.add_systems((
            (|mut a: ResMut<CountA>| { a.0 += 1; })
                .in_set(UpdateSet::Gameplay),
            (|mut b: ResMut<CountB>| { b.0 += 1; })
                .in_set(UpdateSet::Gameplay),
        ));

        schedule.run(&mut world);

        // Both must have run, regardless of order
        assert_eq!(world.resource::<CountA>().0, 1);
        assert_eq!(world.resource::<CountB>().0, 1);
    }

    #[test]
    fn test_render_stage_ordering() {
        let mut world = World::new();
        world.insert_resource(ExecutionOrder::default());

        let mut schedule = Schedule::default();
        configure_render_ordering(&mut schedule);

        schedule.add_systems((
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("acquire");
            })
            .in_set(RenderSet::SurfaceAcquire),
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("draw");
            })
            .in_set(RenderSet::Draw),
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("present");
            })
            .in_set(RenderSet::Present),
        ));

        schedule.run(&mut world);

        let order = world.resource::<ExecutionOrder>();
        assert_eq!(order.0, vec!["acquire", "draw", "present"]);
    }

    #[test]
    fn test_schedule_initialization_validates_graph() {
        // A valid schedule should initialize without panic.
        let mut world = World::new();
        world.insert_resource(ExecutionOrder::default());

        let mut schedule = Schedule::default();
        configure_fixedupdate_ordering(&mut schedule);

        schedule.add_systems(
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("test");
            })
            .in_set(FixedUpdateSet::PhysicsStep),
        );

        // This should not panic — the graph is acyclic.
        schedule.initialize(&mut world);
    }

    #[test]
    fn test_fine_grained_ordering_within_set() {
        let mut world = World::new();
        world.insert_resource(ExecutionOrder::default());

        let mut schedule = Schedule::default();
        schedule.add_systems((
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("compute_damage");
            })
            .in_set(UpdateSet::Gameplay)
            .before(apply_damage_marker),
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("apply_damage");
            })
            .in_set(UpdateSet::Gameplay)
            .in_set(apply_damage_marker)
            .before(check_death_marker),
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("check_death");
            })
            .in_set(UpdateSet::Gameplay)
            .in_set(check_death_marker),
        ));

        schedule.run(&mut world);

        let order = world.resource::<ExecutionOrder>();
        assert_eq!(
            order.0,
            vec!["compute_damage", "apply_damage", "check_death"]
        );
    }
}

// Helper marker sets for fine-grained ordering tests
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct apply_damage_marker;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct check_death_marker;
```
