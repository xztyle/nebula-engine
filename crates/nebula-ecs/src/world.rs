//! World factory function and core resource registration.

use bevy_ecs::prelude::*;

use crate::{CameraRes, ChunkManager, InputState, TimeRes, VoxelRegistry};

/// Registers all core engine resources into the given world with sensible defaults.
///
/// `RenderContext` is intentionally omitted — it is inserted later by
/// nebula-render after GPU initialization.
pub fn register_core_resources(world: &mut World) {
    world.insert_resource(TimeRes::default());
    world.insert_resource(CameraRes::default());
    world.insert_resource(InputState::default());
    world.insert_resource(ChunkManager::new(16));
    world.insert_resource(VoxelRegistry::default());
}

/// Creates and returns a fully initialized ECS world with all engine
/// resources pre-inserted ([`TimeRes`], [`CameraRes`], [`InputState`],
/// [`ChunkManager`], [`VoxelRegistry`]) and default values.
pub fn create_world() -> World {
    let mut world = World::new();
    register_core_resources(&mut world);
    world
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_core_resources_inserts_all() {
        let mut world = World::new();
        register_core_resources(&mut world);

        assert!(world.contains_resource::<TimeRes>());
        assert!(world.contains_resource::<CameraRes>());
        assert!(world.contains_resource::<InputState>());
        assert!(world.contains_resource::<ChunkManager>());
        assert!(world.contains_resource::<VoxelRegistry>());
    }

    #[test]
    fn test_create_world_has_all_resources() {
        let world = create_world();
        assert!(world.contains_resource::<TimeRes>());
        assert!(world.contains_resource::<CameraRes>());
        assert!(world.contains_resource::<InputState>());
        assert!(world.contains_resource::<ChunkManager>());
        assert!(world.contains_resource::<VoxelRegistry>());
    }

    #[test]
    fn test_time_res_defaults() {
        let world = create_world();
        let time = world.resource::<TimeRes>();
        assert_eq!(time.delta, 0.0);
        assert_eq!(time.elapsed, 0.0);
        assert_eq!(time.tick, 0);
    }

    #[test]
    fn test_resmut_allows_mutation() {
        let mut world = create_world();
        let mut time = world.resource_mut::<TimeRes>();
        time.delta = 0.016;
        time.elapsed = 1.5;
        time.tick = 90;

        let time = world.resource::<TimeRes>();
        assert_eq!(time.delta, 0.016);
        assert_eq!(time.elapsed, 1.5);
        assert_eq!(time.tick, 90);
    }

    #[test]
    fn test_multiple_res_reads_are_concurrent() {
        let mut world = create_world();
        {
            let mut time = world.resource_mut::<TimeRes>();
            time.elapsed = 5.0;
        }

        #[derive(Resource, Default)]
        struct ReadA(f64);
        #[derive(Resource, Default)]
        struct ReadB(f64);
        world.insert_resource(ReadA::default());
        world.insert_resource(ReadB::default());

        let mut schedule = Schedule::default();
        schedule.add_systems((
            |time: Res<TimeRes>, mut a: ResMut<ReadA>| {
                a.0 = time.elapsed;
            },
            |time: Res<TimeRes>, mut b: ResMut<ReadB>| {
                b.0 = time.elapsed;
            },
        ));

        schedule.run(&mut world);

        assert_eq!(world.resource::<ReadA>().0, 5.0);
        assert_eq!(world.resource::<ReadB>().0, 5.0);
    }

    #[test]
    #[should_panic]
    fn test_missing_resource_panics_at_runtime() {
        let world = World::new();
        let _time = world.resource::<TimeRes>();
    }
}
