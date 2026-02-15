# Common Query Patterns

## Problem

As the Nebula Engine grows, the same ECS query patterns appear repeatedly across subsystems: "all entities with position and velocity" for movement, "all entities with a mesh and local position" for rendering, "all entities that just received a new component" for initialization. Without a shared vocabulary of query patterns, each subsystem author writes their own version with slightly different type signatures, filter combinations, and optional component handling. This leads to inconsistency, duplicated logic, and subtle bugs when one system forgets a filter (e.g., querying entities without checking `Active`).

Providing documented, tested query patterns — as both reference documentation and reusable type aliases or helper functions — ensures consistency across the engine and reduces the cognitive overhead of writing new systems. These patterns also serve as onboarding material for contributors who are new to bevy_ecs.

## Solution

### Pattern 1: Movement Query

The most fundamental query: entities with both a position and a velocity, used by the physics integration system to update positions each fixed tick.

```rust
use bevy_ecs::prelude::*;

/// Query for all entities that can move: have a WorldPos and Velocity.
/// Used in FixedUpdate to integrate velocity into position.
pub type MovementQuery<'w, 's> = Query<'w, 's, (&'static mut WorldPos, &'static Velocity)>;

fn movement_system(mut query: MovementQuery) {
    for (mut pos, vel) in &mut query {
        pos.0.x += vel.0.x;
        pos.0.y += vel.0.y;
        pos.0.z += vel.0.z;
    }
}
```

To filter out inactive entities, add the `Active` filter:

```rust
fn movement_system_active_only(
    mut query: Query<(&mut WorldPos, &Velocity, &Active)>,
) {
    for (mut pos, vel, active) in &mut query {
        if !active.0 {
            continue;
        }
        pos.0.x += vel.0.x;
        pos.0.y += vel.0.y;
        pos.0.z += vel.0.z;
    }
}
```

Alternatively, use a `With<Active>` filter component when `Active(true)` is the only valid state worth querying (filter at the archetype level rather than per-entity):

```rust
/// Marker component for entities that should participate in simulation.
/// Added/removed rather than toggling a bool, enabling archetype-level filtering.
#[derive(Component)]
pub struct Simulated;

fn movement_system_filtered(
    mut query: Query<(&mut WorldPos, &Velocity), With<Simulated>>,
) {
    for (mut pos, vel) in &mut query {
        pos.0.x += vel.0.x;
        pos.0.y += vel.0.y;
        pos.0.z += vel.0.z;
    }
}
```

### Pattern 2: Rendering Query

Entities that should be drawn: they have a local-space position (computed in PostUpdate), a mesh handle, and are active.

```rust
#[derive(Component)]
pub struct MeshHandle(pub u64); // opaque handle to a GPU mesh

/// Query for all renderable entities.
fn render_query_system(
    query: Query<(Entity, &LocalPos, &MeshHandle, &Rotation, &Scale), With<Active>>,
) {
    for (entity, local_pos, mesh, rotation, scale) in &query {
        // Build model matrix from local_pos, rotation, scale
        // Submit draw call with mesh handle
    }
}
```

Entities without `MeshHandle` are automatically excluded — they never appear in the query results. This is bevy_ecs's archetype filtering at work: only archetypes that contain all queried component types are iterated.

### Pattern 3: Newly Added Entity Query

Detect entities that just received a `WorldPos` component (newly spawned or had the component added dynamically). Used for one-time initialization.

```rust
fn initialize_new_spatial_entities(
    camera: Res<CameraRes>,
    mut query: Query<
        (Entity, &WorldPos, &mut LocalPos),
        Added<WorldPos>,
    >,
) {
    for (entity, world_pos, mut local_pos) in &mut query {
        let offset = world_pos.0 - camera.world_origin;
        local_pos.0 = LocalPosition::new(
            offset.x as f32,
            offset.y as f32,
            offset.z as f32,
        );
        // Additional one-time setup: register in spatial index, etc.
    }
}
```

The `Added<WorldPos>` filter fires exactly once per entity — on the first schedule run after the component was added. Subsequent frames do not match even if the entity still has `WorldPos`.

### Pattern 4: Optional Component Query

Query entities that always have some components but may or may not have others. `Option<&T>` in a query returns `Some(&T)` if the component exists or `None` if it does not.

```rust
fn render_with_optional_name(
    query: Query<(&LocalPos, &MeshHandle, Option<&Name>)>,
) {
    for (local_pos, mesh, name) in &query {
        let label = match name {
            Some(n) => n.0.as_str(),
            None => "<unnamed>",
        };
        // Use label for debug rendering
    }
}
```

This is essential for components like `Name` that are optional debug metadata. The query matches all entities with `LocalPos` and `MeshHandle`, regardless of whether they have a `Name`.

### Pattern 5: Entity Reference Query

Look up a specific entity's components by its `Entity` ID:

```rust
fn camera_follow_system(
    camera: Res<CameraRes>,
    query: Query<&WorldPos>,
) {
    if let Ok(pos) = query.get(camera.entity) {
        // Use the camera entity's WorldPos
    }
}
```

### Pattern 6: Exclusion Filter

Query entities that do **not** have a specific component:

```rust
/// Find entities with WorldPos but without LocalPos — these need
/// LocalPos added before they can be rendered.
fn detect_missing_local_pos(
    query: Query<Entity, (With<WorldPos>, Without<LocalPos>)>,
    mut commands: Commands,
) {
    for entity in &query {
        commands.entity(entity).insert(LocalPos::default());
    }
}
```

### Pattern 7: Combined Changed + With Filter

Process only entities that changed a specific component and have another component:

```rust
fn update_physics_collider_on_scale_change(
    query: Query<(Entity, &Scale), (Changed<Scale>, With<WorldPos>)>,
) {
    for (entity, scale) in &query {
        // Rebuild the physics collider at the new scale
    }
}
```

### Pattern 8: Multi-Component Mutation

Mutate multiple components on the same entity in a single query:

```rust
fn apply_knockback(
    mut query: Query<(&mut WorldPos, &mut Velocity, &KnockbackEffect)>,
) {
    for (mut pos, mut vel, knockback) in &mut query {
        vel.0.x += knockback.direction.x;
        vel.0.y += knockback.direction.y;
        vel.0.z += knockback.direction.z;
        // WorldPos is also mutable if immediate displacement is needed
    }
}

#[derive(Component)]
pub struct KnockbackEffect {
    pub direction: Vec3I128,
}
```

### Helper Functions

Provide utility functions that encapsulate common query-and-act patterns:

```rust
/// Count entities matching a query filter.
pub fn count_entities<F: QueryFilter>(world: &mut World) -> usize {
    world.query_filtered::<Entity, F>().iter(world).count()
}

/// Collect all entity IDs matching a query filter.
pub fn collect_entities<F: QueryFilter>(world: &mut World) -> Vec<Entity> {
    world.query_filtered::<Entity, F>().iter(world).collect()
}

/// Check if an entity has a specific component.
pub fn has_component<T: Component>(world: &World, entity: Entity) -> bool {
    world.get::<T>(entity).is_some()
}
```

## Outcome

After this story is complete:

- Eight documented query patterns cover movement, rendering, initialization, optional components, entity lookup, exclusion, change-filtered, and multi-mutation scenarios
- Type aliases (`MovementQuery`) and helper functions (`count_entities`, `has_component`) reduce boilerplate
- Each pattern is tested and demonstrates correct bevy_ecs usage
- New subsystem authors have a reference for writing correct, efficient queries
- The `Simulated` marker component pattern provides archetype-level filtering as an alternative to runtime `Active` checks

## Demo Integration

**Demo crate:** `nebula-demo`

No visible demo change; the canonical query patterns are established (e.g., `Query<(&Transform, &ChunkData), Changed<ChunkData>>` for dirty chunk detection).

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.15` | `Query`, `With`, `Without`, `Changed`, `Added`, `Option<&T>`, `Commands` |
| `nebula-math` | workspace | `WorldPosition`, `LocalPosition`, `Vec3I128` |

Rust edition 2024. All query patterns use bevy_ecs's built-in query system. No additional dependencies required.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::prelude::*;

    #[derive(Component, Default, Debug, PartialEq)]
    struct MeshHandle(u64);

    #[derive(Component)]
    struct KnockbackEffect {
        direction: Vec3I128,
    }

    #[test]
    fn test_movement_query_matches_entities_with_both_components() {
        let mut world = World::new();

        // Entity with both WorldPos and Velocity — should match
        world.spawn((WorldPos::new(0, 0, 0), Velocity::new(1, 0, 0)));

        // Entity with only WorldPos — should NOT match
        world.spawn(WorldPos::new(5, 5, 5));

        // Entity with only Velocity — should NOT match
        world.spawn(Velocity::new(0, 1, 0));

        let mut query = world.query::<(&WorldPos, &Velocity)>();
        let results: Vec<_> = query.iter(&world).collect();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.0.x, 0);
        assert_eq!(results[0].1.0.x, 1);
    }

    #[test]
    fn test_rendering_query_excludes_entities_without_mesh() {
        let mut world = World::new();

        // Renderable entity: has LocalPos, MeshHandle, and Active
        world.spawn((
            LocalPos::new(1.0, 2.0, 3.0),
            MeshHandle(42),
            Active(true),
        ));

        // Non-renderable: no MeshHandle
        world.spawn((
            LocalPos::new(4.0, 5.0, 6.0),
            Active(true),
        ));

        // Non-renderable: has mesh but inactive (still matches the query
        // if we use With<Active> — Active(false) still satisfies With)
        world.spawn((
            LocalPos::new(7.0, 8.0, 9.0),
            MeshHandle(99),
            Active(false),
        ));

        // Query for renderable entities (LocalPos + MeshHandle)
        let mut query = world.query::<(&LocalPos, &MeshHandle)>();
        let results: Vec<_> = query.iter(&world).collect();

        // Two entities have both LocalPos and MeshHandle
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_added_query_only_fires_once() {
        let mut world = World::new();

        #[derive(Resource, Default)]
        struct AddedCount(u32);
        world.insert_resource(AddedCount::default());

        world.spawn(WorldPos::new(1, 2, 3));

        let mut schedule = Schedule::default();
        schedule.add_systems(
            |mut count: ResMut<AddedCount>,
             query: Query<Entity, Added<WorldPos>>| {
                count.0 += query.iter().count() as u32;
            },
        );

        // Frame 1: Added fires
        schedule.run(&mut world);
        assert_eq!(world.resource::<AddedCount>().0, 1);

        // Frame 2: Added does not fire again
        schedule.run(&mut world);
        assert_eq!(world.resource::<AddedCount>().0, 1);

        // Frame 3: still no re-fire
        schedule.run(&mut world);
        assert_eq!(world.resource::<AddedCount>().0, 1);
    }

    #[test]
    fn test_query_with_option_handles_missing_components() {
        let mut world = World::new();

        // Entity with Name
        world.spawn((LocalPos::new(1.0, 0.0, 0.0), Name::new("named")));

        // Entity without Name
        world.spawn(LocalPos::new(2.0, 0.0, 0.0));

        let mut query = world.query::<(&LocalPos, Option<&Name>)>();
        let results: Vec<_> = query.iter(&world).collect();

        assert_eq!(results.len(), 2);

        let named: Vec<_> = results
            .iter()
            .filter(|(_, name)| name.is_some())
            .collect();
        let unnamed: Vec<_> = results
            .iter()
            .filter(|(_, name)| name.is_none())
            .collect();

        assert_eq!(named.len(), 1);
        assert_eq!(unnamed.len(), 1);
        assert_eq!(named[0].1.unwrap().0, "named");
    }

    #[test]
    fn test_exclusion_filter_without() {
        let mut world = World::new();

        // Entity with WorldPos and LocalPos
        world.spawn((WorldPos::new(1, 0, 0), LocalPos::default()));

        // Entity with WorldPos but WITHOUT LocalPos
        world.spawn(WorldPos::new(2, 0, 0));

        // Entity with WorldPos but WITHOUT LocalPos
        world.spawn(WorldPos::new(3, 0, 0));

        let mut query = world
            .query_filtered::<&WorldPos, (With<WorldPos>, Without<LocalPos>)>();
        let results: Vec<_> = query.iter(&world).collect();

        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_entity_lookup_by_id() {
        let mut world = World::new();
        let entity = world.spawn(WorldPos::new(42, 0, 0)).id();

        let mut query = world.query::<&WorldPos>();
        let result = query.get(&world, entity);

        assert!(result.is_ok());
        assert_eq!(result.unwrap().0.x, 42);
    }

    #[test]
    fn test_entity_lookup_nonexistent() {
        let mut world = World::new();
        let entity = world.spawn(WorldPos::default()).id();
        world.despawn(entity);

        let mut query = world.query::<&WorldPos>();
        let result = query.get(&world, entity);

        assert!(result.is_err());
    }

    #[test]
    fn test_movement_system_updates_position() {
        let mut world = World::new();
        let entity = world
            .spawn((
                WorldPos::new(100, 200, 300),
                Velocity::new(10, -5, 20),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(movement_system);
        schedule.run(&mut world);

        let pos = world.get::<WorldPos>(entity).unwrap();
        assert_eq!(pos.0.x, 110);
        assert_eq!(pos.0.y, 195);
        assert_eq!(pos.0.z, 320);
    }

    #[test]
    fn test_count_entities_helper() {
        let mut world = World::new();
        world.spawn((WorldPos::default(), Velocity::default()));
        world.spawn((WorldPos::default(), Velocity::default()));
        world.spawn(WorldPos::default()); // no velocity

        let count = count_entities::<(With<WorldPos>, With<Velocity>)>(&mut world);
        assert_eq!(count, 2);
    }

    #[test]
    fn test_has_component_helper() {
        let mut world = World::new();
        let entity = world.spawn(WorldPos::default()).id();

        assert!(has_component::<WorldPos>(&world, entity));
        assert!(!has_component::<Velocity>(&world, entity));
    }

    #[test]
    fn test_changed_with_filter_combination() {
        let mut world = World::new();

        #[derive(Resource, Default)]
        struct ChangeCount(u32);
        world.insert_resource(ChangeCount::default());

        // Entity with both Scale and WorldPos
        let entity = world
            .spawn((Scale(1.0), WorldPos::default()))
            .id();

        // Entity with Scale but WITHOUT WorldPos
        world.spawn(Scale(2.0));

        let mut schedule = Schedule::default();
        schedule.add_systems(
            |mut count: ResMut<ChangeCount>,
             query: Query<&Scale, (Changed<Scale>, With<WorldPos>)>| {
                count.0 += query.iter().count() as u32;
            },
        );

        // First run: Added implies Changed, only the entity with WorldPos matches
        schedule.run(&mut world);
        assert_eq!(world.resource::<ChangeCount>().0, 1);

        // Second run: no changes
        schedule.run(&mut world);
        assert_eq!(world.resource::<ChangeCount>().0, 1);

        // Mutate Scale on the entity with WorldPos
        world.get_mut::<Scale>(entity).unwrap().0 = 3.0;

        // Third run: detects the change
        schedule.run(&mut world);
        assert_eq!(world.resource::<ChangeCount>().0, 2);
    }

    #[test]
    fn test_multi_component_mutation() {
        let mut world = World::new();
        let entity = world
            .spawn((
                WorldPos::new(0, 0, 0),
                Velocity::new(0, 0, 0),
                KnockbackEffect {
                    direction: Vec3I128::new(100, 50, 0),
                },
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(
            |mut query: Query<(&mut WorldPos, &mut Velocity, &KnockbackEffect)>| {
                for (mut _pos, mut vel, kb) in &mut query {
                    vel.0.x += kb.direction.x;
                    vel.0.y += kb.direction.y;
                    vel.0.z += kb.direction.z;
                }
            },
        );

        schedule.run(&mut world);

        let vel = world.get::<Velocity>(entity).unwrap();
        assert_eq!(vel.0.x, 100);
        assert_eq!(vel.0.y, 50);
        assert_eq!(vel.0.z, 0);
    }
}
```
