//! Change detection systems leveraging bevy_ecs `Changed<T>` and `Added<T>` filters.
//!
//! These systems avoid redundant computation by only processing entities
//! whose relevant components actually changed since the last system run.

use bevy_ecs::prelude::*;
use nebula_math::LocalPosition;

use crate::{CameraRes, LocalPos, WorldPos};

/// Query filter for entities whose [`WorldPos`] was changed or newly added.
type WorldPosChangedFilter = Or<(Changed<WorldPos>, Added<WorldPos>)>;

/// Recomputes [`LocalPos`] only for entities whose [`WorldPos`] changed
/// or was just added, when the camera has **not** moved this frame.
///
/// **Stage:** PostUpdate.
pub fn update_local_positions_incremental(
    camera: Res<CameraRes>,
    mut query: Query<(&WorldPos, &mut LocalPos), WorldPosChangedFilter>,
) {
    if camera.is_changed() {
        // Camera moved — the full-recompute system handles this case.
        return;
    }
    for (world_pos, mut local_pos) in &mut query {
        let offset = world_pos.0 - camera.world_origin;
        local_pos.0 = LocalPosition::new(offset.x as f32, offset.y as f32, offset.z as f32);
    }
}

/// Recomputes [`LocalPos`] for **all** entities when the camera origin
/// changes. A camera move invalidates every entity's camera-relative
/// position, so a full sweep is required.
///
/// **Stage:** PostUpdate.
pub fn update_all_local_positions_on_camera_move(
    camera: Res<CameraRes>,
    mut query: Query<(&WorldPos, &mut LocalPos)>,
) {
    if !camera.is_changed() {
        return;
    }
    for (world_pos, mut local_pos) in &mut query {
        let offset = world_pos.0 - camera.world_origin;
        local_pos.0 = LocalPosition::new(offset.x as f32, offset.y as f32, offset.z as f32);
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::prelude::*;

    #[derive(Component, Default, Debug, PartialEq)]
    struct Counter(u32);

    #[derive(Resource, Default)]
    struct DetectedChanges(u32);

    #[derive(Resource, Default)]
    struct DetectedAdds(u32);

    #[test]
    fn test_unchanged_component_not_detected() {
        let mut world = World::new();
        world.insert_resource(DetectedChanges::default());

        // Spawn entity with Counter
        world.spawn(Counter(0));

        let mut schedule = Schedule::default();
        schedule.add_systems(
            |mut count: ResMut<DetectedChanges>, query: Query<&Counter, Changed<Counter>>| {
                count.0 += query.iter().count() as u32;
            },
        );

        // First run: Added implies Changed, so it fires
        schedule.run(&mut world);
        assert_eq!(world.resource::<DetectedChanges>().0, 1);

        // Second run: nothing changed, should not fire
        schedule.run(&mut world);
        assert_eq!(world.resource::<DetectedChanges>().0, 1);
    }

    #[test]
    fn test_changed_component_detected() {
        let mut world = World::new();
        world.insert_resource(DetectedChanges::default());

        let entity = world.spawn(Counter(0)).id();

        let mut detect_schedule = Schedule::default();
        detect_schedule.add_systems(
            |mut count: ResMut<DetectedChanges>, query: Query<&Counter, Changed<Counter>>| {
                count.0 += query.iter().count() as u32;
            },
        );

        // First run: detects the initial add
        detect_schedule.run(&mut world);
        assert_eq!(world.resource::<DetectedChanges>().0, 1);

        // Mutate the component
        world.get_mut::<Counter>(entity).unwrap().0 = 42;

        // Second run: should detect the change
        detect_schedule.run(&mut world);
        assert_eq!(world.resource::<DetectedChanges>().0, 2);
    }

    #[test]
    fn test_added_fires_on_first_frame_only() {
        let mut world = World::new();
        world.insert_resource(DetectedAdds::default());

        let mut schedule = Schedule::default();
        schedule.add_systems(
            |mut count: ResMut<DetectedAdds>, query: Query<Entity, Added<Counter>>| {
                count.0 += query.iter().count() as u32;
            },
        );

        // Spawn entity
        world.spawn(Counter(0));

        // First run: Added fires
        schedule.run(&mut world);
        assert_eq!(world.resource::<DetectedAdds>().0, 1);

        // Second run: Added does NOT fire again
        schedule.run(&mut world);
        assert_eq!(world.resource::<DetectedAdds>().0, 1);

        // Third run: still no re-fire
        schedule.run(&mut world);
        assert_eq!(world.resource::<DetectedAdds>().0, 1);
    }

    #[test]
    fn test_detection_resets_each_frame() {
        let mut world = World::new();
        world.insert_resource(DetectedChanges::default());

        let entity = world.spawn(Counter(0)).id();

        let mut schedule = Schedule::default();
        schedule.add_systems(
            |mut count: ResMut<DetectedChanges>, query: Query<&Counter, Changed<Counter>>| {
                count.0 += query.iter().count() as u32;
            },
        );

        // Run 1: initial add detected
        schedule.run(&mut world);
        assert_eq!(world.resource::<DetectedChanges>().0, 1);

        // Run 2: no change, not detected
        schedule.run(&mut world);
        assert_eq!(world.resource::<DetectedChanges>().0, 1);

        // Mutate
        world.get_mut::<Counter>(entity).unwrap().0 = 10;

        // Run 3: change detected
        schedule.run(&mut world);
        assert_eq!(world.resource::<DetectedChanges>().0, 2);

        // Run 4: flag has reset, not detected
        schedule.run(&mut world);
        assert_eq!(world.resource::<DetectedChanges>().0, 2);
    }

    #[test]
    fn test_multiple_changes_per_frame_coalesce() {
        let mut world = World::new();
        world.insert_resource(DetectedChanges::default());

        let entity = world.spawn(Counter(0)).id();

        let mut schedule = Schedule::default();
        schedule.add_systems(
            |mut count: ResMut<DetectedChanges>, query: Query<&Counter, Changed<Counter>>| {
                count.0 += query.iter().count() as u32;
            },
        );

        // Clear the initial add detection
        schedule.run(&mut world);
        let baseline = world.resource::<DetectedChanges>().0;

        // Mutate the component multiple times before running the schedule
        world.get_mut::<Counter>(entity).unwrap().0 = 1;
        world.get_mut::<Counter>(entity).unwrap().0 = 2;
        world.get_mut::<Counter>(entity).unwrap().0 = 3;

        // Run: should detect only ONE change (coalesced)
        schedule.run(&mut world);
        assert_eq!(world.resource::<DetectedChanges>().0, baseline + 1);
    }

    #[test]
    fn test_added_and_changed_both_fire_on_spawn() {
        let mut world = World::new();
        world.insert_resource(DetectedAdds::default());
        world.insert_resource(DetectedChanges::default());

        let mut schedule = Schedule::default();
        schedule.add_systems((
            |mut count: ResMut<DetectedAdds>, query: Query<Entity, Added<Counter>>| {
                count.0 += query.iter().count() as u32;
            },
            |mut count: ResMut<DetectedChanges>, query: Query<&Counter, Changed<Counter>>| {
                count.0 += query.iter().count() as u32;
            },
        ));

        world.spawn(Counter(0));
        schedule.run(&mut world);

        // Both Added and Changed should fire on the first frame
        assert_eq!(world.resource::<DetectedAdds>().0, 1);
        assert_eq!(world.resource::<DetectedChanges>().0, 1);
    }

    #[test]
    fn test_change_detection_independent_per_system() {
        let mut world = World::new();

        #[derive(Resource, Default)]
        struct SystemACount(u32);
        #[derive(Resource, Default)]
        struct SystemBCount(u32);

        world.insert_resource(SystemACount::default());
        world.insert_resource(SystemBCount::default());

        let entity = world.spawn(Counter(0)).id();

        let mut schedule_a = Schedule::default();
        schedule_a.add_systems(
            |mut count: ResMut<SystemACount>, query: Query<&Counter, Changed<Counter>>| {
                count.0 += query.iter().count() as u32;
            },
        );

        let mut schedule_b = Schedule::default();
        schedule_b.add_systems(
            |mut count: ResMut<SystemBCount>, query: Query<&Counter, Changed<Counter>>| {
                count.0 += query.iter().count() as u32;
            },
        );

        // Both systems see the initial add
        schedule_a.run(&mut world);
        schedule_b.run(&mut world);
        assert_eq!(world.resource::<SystemACount>().0, 1);
        assert_eq!(world.resource::<SystemBCount>().0, 1);

        // Mutate
        world.get_mut::<Counter>(entity).unwrap().0 = 99;

        // Only run system A — it sees the change
        schedule_a.run(&mut world);
        assert_eq!(world.resource::<SystemACount>().0, 2);

        // System B has not run yet — it should still see the change
        schedule_b.run(&mut world);
        assert_eq!(world.resource::<SystemBCount>().0, 2);
    }
}
