# Change Detection

## Problem

The Nebula Engine simulates planets with millions of voxels, thousands of entities, and complex spatial data structures. Recomputing everything every frame is prohibitively expensive. When a single entity moves, only its `LocalPos` needs to be recomputed — not every entity's. When a single voxel changes in one chunk, only that chunk's mesh needs to be rebuilt — not every chunk in the world. Without change detection, the engine must choose between recomputing everything (wasting 99% of CPU time on unchanged data) or building a bespoke dirty-flag system for every component type (fragile, error-prone, and duplicating work that the ECS already tracks internally).

bevy_ecs provides built-in change detection via `Changed<T>` and `Added<T>` query filters. These track which components were mutated or inserted since the last time the observing system ran. Leveraging this built-in mechanism eliminates the need for manual dirty flags while providing per-component, per-entity granularity with zero additional memory overhead beyond what bevy_ecs already maintains.

## Solution

### bevy_ecs Change Detection Primitives

bevy_ecs automatically tracks two kinds of changes for every component type:

1. **`Changed<T>`** — True if the component `T` was modified (via `&mut T` or `Mut<T>`) since the last time the system checking `Changed<T>` ran. Accessing a component through `&mut T` in a query marks it as changed, even if the value was not actually modified. This is by design — bevy_ecs tracks access, not value equality.

2. **`Added<T>`** — True if the component `T` was added to the entity since the last time the system checking `Added<T>` ran. This fires only once, on the first frame the system observes the new component. It does not fire on subsequent frames even if the component is mutated.

Both filters are used as query filter parameters:

```rust
fn system_example(
    query: Query<(Entity, &WorldPos), Changed<WorldPos>>,
) {
    for (entity, pos) in &query {
        // Only entities whose WorldPos changed this frame
    }
}
```

### Key Engine Use Cases

#### WorldPos -> LocalPos Update

The most critical use case: only recompute `LocalPos` for entities whose `WorldPos` changed or whose `WorldPos` was just added.

```rust
fn update_local_positions_incremental(
    camera: Res<CameraRes>,
    mut query: Query<
        (&WorldPos, &mut LocalPos),
        Or<(Changed<WorldPos>, Added<WorldPos>)>,
    >,
) {
    for (world_pos, mut local_pos) in &mut query {
        let offset = world_pos.0 - camera.world_origin;
        local_pos.0 = LocalPosition::new(
            offset.x as f32,
            offset.y as f32,
            offset.z as f32,
        );
    }
}
```

Note: when the camera moves, `CameraRes` changes and *all* `LocalPos` values must be recomputed. A separate system detects `CameraRes` changes and performs a full recompute:

```rust
fn update_all_local_positions_on_camera_move(
    camera: Res<CameraRes>,
    mut query: Query<(&WorldPos, &mut LocalPos)>,
) {
    if !camera.is_changed() {
        return;
    }
    for (world_pos, mut local_pos) in &mut query {
        let offset = world_pos.0 - camera.world_origin;
        local_pos.0 = LocalPosition::new(
            offset.x as f32,
            offset.y as f32,
            offset.z as f32,
        );
    }
}
```

#### Voxel Change -> Mesh Rebuild

When voxel data changes in a chunk, only that chunk's mesh needs to be rebuilt:

```rust
/// Marker component on chunk entities indicating their voxel data.
#[derive(Component)]
struct ChunkVoxelData { /* ... */ }

/// Marker component for the chunk's generated mesh.
#[derive(Component)]
struct ChunkMesh { /* ... */ }

fn remesh_changed_chunks(
    mut query: Query<
        (Entity, &ChunkVoxelData, &mut ChunkMesh),
        Changed<ChunkVoxelData>,
    >,
) {
    for (entity, voxels, mut mesh) in &mut query {
        // Only rebuild meshes for chunks whose voxels actually changed
        *mesh = generate_mesh(voxels);
    }
}
```

#### Entity Initialization on Add

When a new entity with `WorldPos` is spawned, initialize its `LocalPos` on the first frame:

```rust
fn initialize_new_entities(
    camera: Res<CameraRes>,
    mut query: Query<
        (&WorldPos, &mut LocalPos),
        Added<WorldPos>,
    >,
) {
    for (world_pos, mut local_pos) in &mut query {
        let offset = world_pos.0 - camera.world_origin;
        local_pos.0 = LocalPosition::new(
            offset.x as f32,
            offset.y as f32,
            offset.z as f32,
        );
    }
}
```

### Change Detection Lifecycle

1. **Frame N:** Entity is spawned with `WorldPos`. Both `Added<WorldPos>` and `Changed<WorldPos>` are true.
2. **Frame N+1:** No modification to `WorldPos`. Both `Added<WorldPos>` and `Changed<WorldPos>` are false.
3. **Frame N+2:** A system writes `&mut WorldPos`. `Changed<WorldPos>` is true. `Added<WorldPos>` is false.
4. **Frame N+3:** No modification. Both are false again.

Change flags are reset per-system, not globally. If system A checks `Changed<WorldPos>` and system B also checks `Changed<WorldPos>`, each system sees the change independently based on when it last ran. This prevents one system's observation from "consuming" the change flag before another system sees it.

### Avoiding False Positives

bevy_ecs marks a component as changed whenever it is accessed mutably, even if the value is unchanged. To minimize false positives:

```rust
// BAD: Always marks Velocity as changed even if zero
fn apply_gravity_bad(mut query: Query<&mut Velocity>) {
    for mut vel in &mut query {
        vel.0.y -= 9810; // 9.81 m/s^2 in mm/tick^2
    }
}

// GOOD: Only access mutably when there is something to change
fn apply_gravity_good(
    mut query: Query<&mut Velocity, With<AffectedByGravity>>,
) {
    for mut vel in &mut query {
        // DerefMut triggers changed flag — this is fine because
        // we genuinely are changing the value.
        vel.0.y -= 9810;
    }
}
```

For cases where mutation is conditional:

```rust
fn conditional_update(mut query: Query<&mut Health>) {
    for mut health in &mut query {
        if health.0 < 100 {
            // Only triggers change detection if we actually modify
            health.0 += 1;
        }
        // If health >= 100, the Mut<Health> is not dereferenced mutably,
        // so change detection is NOT triggered. Use `health.bypass_change_detection()`
        // or simply don't deref_mut.
    }
}
```

### Coalescing Multiple Changes

If a component is mutated multiple times within the same system or across multiple systems in the same stage, the `Changed<T>` filter in the next stage sees it as a single change. Change detection does not count the number of mutations — it is a boolean flag: changed or not changed since the observing system last ran.

## Outcome

After this story is complete:

- The `update_local_positions_incremental` system only processes entities whose `WorldPos` actually changed, not all entities
- The `remesh_changed_chunks` system only rebuilds meshes for chunks with modified voxel data
- New entities are detected via `Added<T>` and initialized exactly once
- Change detection resets automatically each frame per-system, requiring no manual flag management
- Multiple mutations within a frame coalesce into a single detected change
- The engine avoids redundant computation for the vast majority of entities that do not change each frame

## Demo Integration

**Demo crate:** `nebula-demo`

The demo only re-meshes chunks whose voxel data component has been mutated. The console logs `Changed: 1 chunk, skipped: 24`.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.15` | Built-in change detection: `Changed<T>`, `Added<T>`, `Mut<T>` |
| `nebula-math` | workspace | `WorldPosition`, `LocalPosition` for transform systems |

Rust edition 2024. Change detection is a zero-cost feature of bevy_ecs — it uses bitflags already stored in the archetype metadata, adding no per-component memory overhead.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
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
            |mut count: ResMut<DetectedChanges>,
             query: Query<&Counter, Changed<Counter>>| {
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
            |mut count: ResMut<DetectedChanges>,
             query: Query<&Counter, Changed<Counter>>| {
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
            |mut count: ResMut<DetectedAdds>,
             query: Query<Entity, Added<Counter>>| {
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
            |mut count: ResMut<DetectedChanges>,
             query: Query<&Counter, Changed<Counter>>| {
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
            |mut count: ResMut<DetectedChanges>,
             query: Query<&Counter, Changed<Counter>>| {
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
            |mut count: ResMut<DetectedAdds>,
             query: Query<Entity, Added<Counter>>| {
                count.0 += query.iter().count() as u32;
            },
            |mut count: ResMut<DetectedChanges>,
             query: Query<&Counter, Changed<Counter>>| {
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
            |mut count: ResMut<SystemACount>,
             query: Query<&Counter, Changed<Counter>>| {
                count.0 += query.iter().count() as u32;
            },
        );

        let mut schedule_b = Schedule::default();
        schedule_b.add_systems(
            |mut count: ResMut<SystemBCount>,
             query: Query<&Counter, Changed<Counter>>| {
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
```
