# Entity Lifecycle

## Problem

Entities must be created and destroyed throughout the simulation: projectiles spawn on fire, enemies despawn on death, chunks load and unload as the player moves, particles burst into existence and fade away. Spawning or despawning an entity mid-system-execution is dangerous because it invalidates active queries and iterators. If a physics system is iterating over all entities with `WorldPos` and `Velocity`, and a gameplay system simultaneously despawns one of those entities, the physics system may read freed memory or skip an entity.

bevy_ecs provides `Commands` for deferred structural changes, but the engine needs higher-level spawn/despawn utilities that bundle common patterns: spawning an entity with a standard set of components, queuing despawns for end-of-stage application, and ensuring that entity IDs are not immediately reused (to prevent stale ID references from resolving to a completely different entity).

## Solution

### Immediate Spawn (Outside Systems)

For setup code and tests that run outside the schedule (e.g., during world initialization), provide a direct spawn helper:

```rust
use bevy_ecs::prelude::*;

/// Spawns an entity with the given component bundle and returns its Entity ID.
/// This function takes `&mut World` directly and is intended for use outside
/// of system execution (setup, tests, editor commands).
pub fn spawn_entity<B: Bundle>(world: &mut World, bundle: B) -> Entity {
    world.spawn(bundle).id()
}

/// Despawns an entity immediately. Safe to call with a nonexistent entity —
/// returns `false` if the entity was already despawned or never existed.
pub fn despawn_entity(world: &mut World, entity: Entity) -> bool {
    world.despawn(entity)
}
```

### Deferred Spawn Queue

During system execution, structural changes must be deferred. The engine provides a `SpawnQueue` resource that collects spawn requests and applies them between stages:

```rust
/// A type-erased spawn request. Stores a closure that, when called with
/// `&mut World`, spawns the entity and returns its ID.
type SpawnFn = Box<dyn FnOnce(&mut World) -> Entity + Send + Sync>;

#[derive(Resource, Default)]
pub struct SpawnQueue {
    pending: Vec<SpawnFn>,
}

impl SpawnQueue {
    /// Queue a spawn request. The entity will be created when the queue
    /// is flushed (between stages).
    pub fn enqueue<B: Bundle + Send + Sync + 'static>(&mut self, bundle: B) {
        self.pending.push(Box::new(move |world| {
            world.spawn(bundle).id()
        }));
    }

    /// Apply all pending spawns. Returns the Entity IDs of the newly
    /// created entities.
    pub fn flush(&mut self, world: &mut World) -> Vec<Entity> {
        let requests: Vec<SpawnFn> = self.pending.drain(..).collect();
        requests.into_iter().map(|f| f(world)).collect()
    }

    /// Returns the number of pending spawn requests.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}
```

### Deferred Despawn Queue

Similarly, the `DespawnQueue` collects entities to remove:

```rust
#[derive(Resource, Default)]
pub struct DespawnQueue {
    pending: Vec<Entity>,
}

impl DespawnQueue {
    /// Queue an entity for despawning. Duplicate entries are allowed
    /// and handled gracefully (the second despawn is a no-op).
    pub fn enqueue(&mut self, entity: Entity) {
        self.pending.push(entity);
    }

    /// Despawn all queued entities. Returns the count of entities
    /// that were actually despawned (excludes already-despawned duplicates).
    pub fn flush(&mut self, world: &mut World) -> usize {
        let entities: Vec<Entity> = self.pending.drain(..).collect();
        entities
            .into_iter()
            .filter(|&e| world.despawn(e))
            .count()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}
```

### Queue Flush System

A system registered at the boundary between stages flushes both queues. This runs as the first system in each stage (or as a dedicated flush pass between stages):

```rust
fn flush_entity_queues(world: &mut World) {
    // Flush spawns first so that newly spawned entities can be
    // immediately referenced by systems in the next stage.
    world.resource_scope(|world, mut spawn_queue: Mut<SpawnQueue>| {
        spawn_queue.flush(world);
    });

    world.resource_scope(|world, mut despawn_queue: Mut<DespawnQueue>| {
        despawn_queue.flush(world);
    });
}
```

This exclusive system (`fn(world: &mut World)`) has full world access and runs outside the parallel executor, ensuring no other system is active when structural changes occur.

### Usage in Systems

Systems that need to spawn or despawn entities write to the queue resources:

```rust
fn projectile_system(
    mut despawn_queue: ResMut<DespawnQueue>,
    query: Query<(Entity, &WorldPos, &Lifetime)>,
) {
    for (entity, _pos, lifetime) in &query {
        if lifetime.is_expired() {
            despawn_queue.enqueue(entity);
        }
    }
}

fn enemy_spawner_system(
    mut spawn_queue: ResMut<SpawnQueue>,
    time: Res<TimeRes>,
) {
    if time.tick % 300 == 0 { // Every 5 seconds at 60 Hz
        spawn_queue.enqueue((
            WorldPos::new(0, 0, 10_000),
            Velocity::default(),
            Rotation::default(),
            Active(true),
            Name::new("enemy"),
        ));
    }
}
```

### bevy_ecs Commands Integration

Systems can also use bevy_ecs's built-in `Commands` for deferred operations. The `SpawnQueue` and `DespawnQueue` are engine-level abstractions that provide additional features (batch flush, entity ID collection, count tracking). `Commands` remain available for one-off deferred operations:

```rust
fn spawn_via_commands(mut commands: Commands) {
    commands.spawn((WorldPos::default(), Active(true)));
}
```

Commands are automatically applied by bevy_ecs between system executions within the same stage. The engine's queues provide cross-stage deferred operations with explicit flush points.

### Entity ID Reuse Policy

bevy_ecs uses generational entity IDs: each `Entity` contains an index and a generation counter. When an entity is despawned, the index is recycled but the generation is incremented. Any stale `Entity` handle still holding the old generation will fail to resolve. This prevents the dangling-reference problem without requiring the engine to implement its own ID recycling.

The engine does not impose additional restrictions on ID reuse timing — bevy_ecs's generational approach is sufficient. However, the `DespawnQueue` introduces a one-stage delay between the despawn request and the actual removal, giving systems in the current stage time to finish iterating.

## Outcome

After this story is complete:

- `spawn_entity(world, bundle)` creates entities immediately for setup and test code
- `despawn_entity(world, entity)` removes entities immediately and handles nonexistent entities gracefully
- `SpawnQueue` and `DespawnQueue` resources enable deferred spawning/despawning during system execution
- Queue flush happens between stages via an exclusive system
- Generational entity IDs prevent stale handle resolution
- Systems can use either the engine queues or bevy_ecs `Commands` for deferred operations
- Despawning an already-despawned entity is safe (no panic, returns false)

## Demo Integration

**Demo crate:** `nebula-demo`

Chunks are spawned and despawned as entities when loading/unloading. The entity count in the title fluctuates as the camera moves.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.15` | World, Entity, Commands, Bundle, resource management |

Rust edition 2024. No additional dependencies beyond `bevy_ecs`. The `SpawnQueue` uses `Box<dyn FnOnce>` for type erasure, which is a standard library feature.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::prelude::*;

    #[derive(Component, Debug, PartialEq)]
    struct Health(u32);

    #[derive(Component, Debug, PartialEq)]
    struct Tag(&'static str);

    #[test]
    fn test_spawn_creates_entity_with_components() {
        let mut world = World::new();
        let entity = spawn_entity(&mut world, (
            WorldPos::new(10, 20, 30),
            Health(100),
            Tag("player"),
        ));

        assert!(world.get_entity(entity).is_ok());
        assert_eq!(world.get::<WorldPos>(entity).unwrap().0.x, 10);
        assert_eq!(world.get::<Health>(entity).unwrap().0, 100);
        assert_eq!(world.get::<Tag>(entity).unwrap().0, "player");
    }

    #[test]
    fn test_despawn_removes_entity() {
        let mut world = World::new();
        let entity = spawn_entity(&mut world, WorldPos::default());

        assert!(world.get_entity(entity).is_ok());
        let result = despawn_entity(&mut world, entity);
        assert!(result);
        assert!(world.get_entity(entity).is_err());
    }

    #[test]
    fn test_despawning_nonexistent_entity_is_safe() {
        let mut world = World::new();
        let entity = spawn_entity(&mut world, WorldPos::default());
        despawn_entity(&mut world, entity);

        // Despawning again should not panic, just return false
        let result = despawn_entity(&mut world, entity);
        assert!(!result);
    }

    #[test]
    fn test_deferred_spawn_applies_between_stages() {
        let mut world = World::new();
        world.insert_resource(SpawnQueue::default());
        world.insert_resource(DespawnQueue::default());
        world.insert_resource(TimeRes::default());

        let mut schedules = EngineSchedules::new();

        // In Update, queue a spawn
        schedules.add_system(
            EngineSchedule::Update,
            |mut queue: ResMut<SpawnQueue>| {
                queue.enqueue((WorldPos::new(42, 0, 0), Health(50)));
            },
        );

        // Register flush between stages
        schedules.add_system(EngineSchedule::PostUpdate, flush_entity_queues);

        schedules.run(&mut world, 1.0 / 60.0);

        // The entity should now exist
        let mut query = world.query::<(&WorldPos, &Health)>();
        let results: Vec<_> = query.iter(&world).collect();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.0.x, 42);
        assert_eq!(results[0].1.0, 50);
    }

    #[test]
    fn test_deferred_despawn_applies_between_stages() {
        let mut world = World::new();
        let entity = spawn_entity(&mut world, WorldPos::default());

        world.insert_resource(SpawnQueue::default());
        world.insert_resource(DespawnQueue::default());
        world.insert_resource(TimeRes::default());

        let target = entity;
        let mut schedules = EngineSchedules::new();

        // In Update, queue the despawn
        schedules.add_system(
            EngineSchedule::Update,
            move |mut queue: ResMut<DespawnQueue>| {
                queue.enqueue(target);
            },
        );

        schedules.add_system(EngineSchedule::PostUpdate, flush_entity_queues);

        schedules.run(&mut world, 1.0 / 60.0);

        // The entity should be gone
        assert!(world.get_entity(entity).is_err());
    }

    #[test]
    fn test_entity_ids_are_not_reused_immediately() {
        let mut world = World::new();
        let entity_a = spawn_entity(&mut world, WorldPos::default());
        despawn_entity(&mut world, entity_a);

        let entity_b = spawn_entity(&mut world, WorldPos::default());

        // Even if the index is reused, the generation must differ
        // so the old Entity handle does not resolve to the new entity.
        assert_ne!(entity_a, entity_b);
        assert!(world.get_entity(entity_a).is_err());
        assert!(world.get_entity(entity_b).is_ok());
    }

    #[test]
    fn test_spawn_queue_reports_pending_count() {
        let mut queue = SpawnQueue::default();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);

        queue.enqueue(WorldPos::default());
        queue.enqueue(WorldPos::default());
        assert_eq!(queue.len(), 2);
        assert!(!queue.is_empty());
    }

    #[test]
    fn test_despawn_queue_handles_duplicates() {
        let mut world = World::new();
        let entity = spawn_entity(&mut world, WorldPos::default());

        let mut queue = DespawnQueue::default();
        queue.enqueue(entity);
        queue.enqueue(entity); // duplicate

        let count = queue.flush(&mut world);
        // Only one actual despawn should succeed
        assert_eq!(count, 1);
    }

    #[test]
    fn test_flush_clears_queues() {
        let mut world = World::new();
        let mut spawn_queue = SpawnQueue::default();
        spawn_queue.enqueue(WorldPos::default());
        spawn_queue.enqueue(WorldPos::default());

        let spawned = spawn_queue.flush(&mut world);
        assert_eq!(spawned.len(), 2);
        assert!(spawn_queue.is_empty());

        let mut despawn_queue = DespawnQueue::default();
        for e in &spawned {
            despawn_queue.enqueue(*e);
        }
        let removed = despawn_queue.flush(&mut world);
        assert_eq!(removed, 2);
        assert!(despawn_queue.is_empty());
    }
}
```
