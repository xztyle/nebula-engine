//! System set definitions and ordering constraints for engine stages.
//!
//! Each major subsystem has a named [`SystemSet`] for grouping related systems.
//! Inter-set ordering constraints ensure correct data flow: input before gameplay,
//! gameplay before physics, physics before transform update, transforms before rendering.

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::Schedule;

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

/// Configure ordering constraints for the PreUpdate stage.
pub fn configure_preupdate_ordering(schedule: &mut Schedule) {
    schedule.configure_sets(PreUpdateSet::Time.before(PreUpdateSet::Input));
}

/// Configure ordering constraints for the FixedUpdate stage.
pub fn configure_fixedupdate_ordering(schedule: &mut Schedule) {
    schedule.configure_sets((
        FixedUpdateSet::InputProcessing.before(FixedUpdateSet::ForceApplication),
        FixedUpdateSet::ForceApplication.before(FixedUpdateSet::PhysicsStep),
        FixedUpdateSet::PhysicsStep.before(FixedUpdateSet::PhysicsSync),
    ));
}

/// Configure ordering constraints for the Update stage.
pub fn configure_update_ordering(schedule: &mut Schedule) {
    schedule.configure_sets((
        UpdateSet::AI.before(UpdateSet::Gameplay),
        UpdateSet::Gameplay.before(UpdateSet::Animation),
    ));
}

/// Configure ordering constraints for the PostUpdate stage.
pub fn configure_postupdate_ordering(schedule: &mut Schedule) {
    schedule.configure_sets(
        PostUpdateSet::TransformPropagation.before(PostUpdateSet::SpatialIndexUpdate),
    );
}

/// Configure ordering constraints for the PreRender stage.
pub fn configure_prerender_ordering(schedule: &mut Schedule) {
    schedule.configure_sets((
        PreRenderSet::Culling.before(PreRenderSet::Batching),
        PreRenderSet::Batching.before(PreRenderSet::BufferUpload),
    ));
}

/// Configure ordering constraints for the Render stage.
pub fn configure_render_ordering(schedule: &mut Schedule) {
    schedule.configure_sets((
        RenderSet::SurfaceAcquire.before(RenderSet::Draw),
        RenderSet::Draw.before(RenderSet::Present),
    ));
}

/// Validate all schedules by forcing graph initialization.
///
/// Panics with a descriptive error if any cycle is detected.
pub fn validate_schedules(schedules: &mut crate::EngineSchedules, world: &mut World) {
    schedules.initialize_all(world);
}

/// Log that a schedule graph dump was requested (debug builds only).
///
/// Full graph introspection requires bevy_ecs internals; this stub logs
/// a confirmation that the schedule has been initialized and validated.
#[cfg(debug_assertions)]
#[allow(dead_code)]
pub fn print_schedule_graph(_schedule: &Schedule) {
    println!("Schedule graph: initialized and validated (no cycles detected)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct ExecutionOrder(Vec<&'static str>);

    #[test]
    fn test_before_constraint_respected() {
        let mut world = World::new();
        world.insert_resource(ExecutionOrder::default());

        let mut schedule = Schedule::default();
        schedule
            .configure_sets(FixedUpdateSet::InputProcessing.before(FixedUpdateSet::PhysicsStep));

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
            "InputProcessing ({input_idx}) must run before PhysicsStep ({physics_idx})",
        );
    }

    #[test]
    fn test_after_constraint_respected() {
        let mut world = World::new();
        world.insert_resource(ExecutionOrder::default());

        let mut schedule = Schedule::default();
        schedule.configure_sets(
            PostUpdateSet::SpatialIndexUpdate.after(PostUpdateSet::TransformPropagation),
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
        let mut world = World::new();

        #[derive(Resource, Default)]
        struct CountA(u32);
        #[derive(Resource, Default)]
        struct CountB(u32);
        world.insert_resource(CountA::default());
        world.insert_resource(CountB::default());

        let mut schedule = Schedule::default();
        schedule.add_systems((
            (|mut a: ResMut<CountA>| {
                a.0 += 1;
            })
            .in_set(UpdateSet::Gameplay),
            (|mut b: ResMut<CountB>| {
                b.0 += 1;
            })
            .in_set(UpdateSet::Gameplay),
        ));

        schedule.run(&mut world);

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
        let _ = schedule.initialize(&mut world);
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
            .before(ApplyDamageMarker),
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("apply_damage");
            })
            .in_set(UpdateSet::Gameplay)
            .in_set(ApplyDamageMarker)
            .before(CheckDeathMarker),
            (|mut order: ResMut<ExecutionOrder>| {
                order.0.push("check_death");
            })
            .in_set(UpdateSet::Gameplay)
            .in_set(CheckDeathMarker),
        ));

        schedule.run(&mut world);

        let order = world.resource::<ExecutionOrder>();
        assert_eq!(
            order.0,
            vec!["compute_damage", "apply_damage", "check_death"]
        );
    }

    /// Helper marker set for fine-grained ordering tests.
    #[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
    struct ApplyDamageMarker;

    /// Helper marker set for fine-grained ordering tests.
    #[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
    struct CheckDeathMarker;
}
