//! Core engine systems registered into the stage pipeline.
//!
//! Each system documents which stage it belongs to and what data it
//! reads/writes, following the stage contract specification.

use bevy_ecs::prelude::*;
use nebula_math::LocalPosition;

use crate::{CameraRes, LocalPos, WorldPos};

/// Computes camera-relative [`LocalPos`] from each entity's [`WorldPos`]
/// and the current [`CameraRes::world_origin`].
///
/// **Stage:** PostUpdate (writes `LocalPos`, reads `WorldPos` and `CameraRes`).
pub fn update_local_positions(
    camera: Res<CameraRes>,
    mut query: Query<(&WorldPos, &mut LocalPos)>,
) {
    for (world_pos, mut local_pos) in &mut query {
        let offset = world_pos.0 - camera.world_origin;
        local_pos.0 = LocalPosition::new(offset.x as f32, offset.y as f32, offset.z as f32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EngineSchedule, EngineSchedules, TimeRes};
    use nebula_math::WorldPosition;

    #[derive(Resource, Default)]
    struct StageOrder(Vec<&'static str>);

    #[test]
    fn test_postupdate_runs_after_update() {
        let mut world = World::new();
        world.insert_resource(StageOrder::default());
        world.insert_resource(TimeRes::default());

        let mut schedules = EngineSchedules::new();
        schedules.add_system(EngineSchedule::Update, |mut order: ResMut<StageOrder>| {
            order.0.push("Update");
        });
        schedules.add_system(
            EngineSchedule::PostUpdate,
            |mut order: ResMut<StageOrder>| {
                order.0.push("PostUpdate");
            },
        );

        schedules.run(&mut world, 1.0 / 60.0);

        let order = world.resource::<StageOrder>();
        let update_idx = order.0.iter().position(|s| *s == "Update").unwrap();
        let post_idx = order.0.iter().position(|s| *s == "PostUpdate").unwrap();
        assert!(update_idx < post_idx, "PostUpdate must run after Update");
    }

    #[test]
    fn test_prerender_runs_after_postupdate() {
        let mut world = World::new();
        world.insert_resource(StageOrder::default());
        world.insert_resource(TimeRes::default());

        let mut schedules = EngineSchedules::new();
        schedules.add_system(
            EngineSchedule::PostUpdate,
            |mut order: ResMut<StageOrder>| {
                order.0.push("PostUpdate");
            },
        );
        schedules.add_system(
            EngineSchedule::PreRender,
            |mut order: ResMut<StageOrder>| {
                order.0.push("PreRender");
            },
        );

        schedules.run(&mut world, 1.0 / 60.0);

        let order = world.resource::<StageOrder>();
        let post_idx = order.0.iter().position(|s| *s == "PostUpdate").unwrap();
        let pre_render_idx = order.0.iter().position(|s| *s == "PreRender").unwrap();
        assert!(post_idx < pre_render_idx);
    }

    #[test]
    fn test_parallel_systems_no_conflicting_writes() {
        let mut world = World::new();
        world.insert_resource(TimeRes::default());

        #[derive(Resource, Default)]
        struct OutputA(f64);
        #[derive(Resource, Default)]
        struct OutputB(f64);
        world.insert_resource(OutputA::default());
        world.insert_resource(OutputB::default());

        let mut schedules = EngineSchedules::new();
        schedules.add_system(
            EngineSchedule::Update,
            (
                |time: Res<TimeRes>, mut a: ResMut<OutputA>| {
                    a.0 = time.elapsed;
                },
                |time: Res<TimeRes>, mut b: ResMut<OutputB>| {
                    b.0 = time.elapsed;
                },
            ),
        );

        // This must not panic — the systems share only Res<TimeRes> (read)
        // and write to disjoint resources.
        schedules.run(&mut world, 1.0 / 60.0);
    }

    #[test]
    fn test_fixed_update_respects_timestep_budget() {
        let mut world = World::new();
        world.insert_resource(TimeRes::default());

        #[derive(Resource, Default)]
        struct SimTime(f64);
        world.insert_resource(SimTime(0.0));

        let mut schedules = EngineSchedules::new();
        schedules.add_system(EngineSchedule::FixedUpdate, |mut sim: ResMut<SimTime>| {
            sim.0 += 1.0 / 60.0;
        });

        for _ in 0..60 {
            schedules.run(&mut world, 1.0 / 60.0);
        }

        let sim = world.resource::<SimTime>();
        assert!(
            (sim.0 - 1.0).abs() < 1e-9,
            "60 ticks at 1/60 should equal 1.0 second, got {}",
            sim.0
        );
    }

    #[test]
    fn test_local_pos_updated_from_world_pos_in_postupdate() {
        let mut world = World::new();
        world.insert_resource(TimeRes::default());
        world.insert_resource(CameraRes {
            entity: Entity::PLACEHOLDER,
            world_origin: WorldPosition::new(1000, 2000, 3000),
        });

        let entity = world
            .spawn((WorldPos::new(1100, 2200, 3300), LocalPos::default()))
            .id();

        let mut schedules = EngineSchedules::new();
        schedules.add_system(EngineSchedule::PostUpdate, update_local_positions);

        schedules.run(&mut world, 1.0 / 60.0);

        let local = world.get::<LocalPos>(entity).unwrap();
        assert_eq!(local.0.x, 100.0);
        assert_eq!(local.0.y, 200.0);
        assert_eq!(local.0.z, 300.0);
    }

    #[test]
    fn test_render_runs_last() {
        let mut world = World::new();
        world.insert_resource(StageOrder::default());
        world.insert_resource(TimeRes::default());

        let mut schedules = EngineSchedules::new();
        for (stage, name) in [
            (EngineSchedule::PreUpdate, "PreUpdate"),
            (EngineSchedule::FixedUpdate, "FixedUpdate"),
            (EngineSchedule::Update, "Update"),
            (EngineSchedule::PostUpdate, "PostUpdate"),
            (EngineSchedule::PreRender, "PreRender"),
            (EngineSchedule::Render, "Render"),
        ] {
            let n = name;
            schedules.add_system(stage, move |mut order: ResMut<StageOrder>| {
                order.0.push(n);
            });
        }

        schedules.run(&mut world, 1.0 / 60.0);

        let order = world.resource::<StageOrder>();
        assert_eq!(*order.0.last().unwrap(), "Render");
    }
}
