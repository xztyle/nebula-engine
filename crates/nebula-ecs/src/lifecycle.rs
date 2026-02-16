//! Entity lifecycle utilities: immediate spawn/despawn and deferred queues.
//!
//! Provides [`spawn_entity`] and [`despawn_entity`] for direct world access
//! (setup, tests), plus [`SpawnQueue`] and [`DespawnQueue`] resources for
//! deferred structural changes during system execution.

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

/// A type-erased spawn request. Stores a closure that, when called with
/// `&mut World`, spawns the entity and returns its ID.
type SpawnFn = Box<dyn FnOnce(&mut World) -> Entity + Send + Sync>;

/// Deferred spawn queue that collects spawn requests during system execution
/// and applies them between stages.
#[derive(Resource, Default)]
pub struct SpawnQueue {
    pending: Vec<SpawnFn>,
}

impl SpawnQueue {
    /// Queue a spawn request. The entity will be created when the queue
    /// is flushed (between stages).
    pub fn enqueue<B: Bundle + Send + Sync + 'static>(&mut self, bundle: B) {
        self.pending
            .push(Box::new(move |world| world.spawn(bundle).id()));
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

    /// Returns `true` if there are no pending spawn requests.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// Deferred despawn queue that collects entities to remove during system
/// execution and applies the removals between stages.
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
        entities.into_iter().filter(|&e| world.despawn(e)).count()
    }

    /// Returns the number of pending despawn requests.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Returns `true` if there are no pending despawn requests.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// Exclusive system that flushes both [`SpawnQueue`] and [`DespawnQueue`].
///
/// Register this in [`PostUpdate`](crate::EngineSchedule::PostUpdate) (or
/// between stages) so structural changes are applied safely outside the
/// parallel executor.
pub fn flush_entity_queues(world: &mut World) {
    world.resource_scope(|world, mut spawn_queue: Mut<SpawnQueue>| {
        spawn_queue.flush(world);
    });

    world.resource_scope(|world, mut despawn_queue: Mut<DespawnQueue>| {
        despawn_queue.flush(world);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EngineSchedule, EngineSchedules, TimeRes, WorldPos};

    #[derive(Component, Debug, PartialEq)]
    struct Health(u32);

    #[derive(Component, Debug, PartialEq)]
    struct Tag(&'static str);

    #[test]
    fn test_spawn_creates_entity_with_components() {
        let mut world = World::new();
        let entity = spawn_entity(
            &mut world,
            (WorldPos::new(10, 20, 30), Health(100), Tag("player")),
        );

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

        schedules.add_system(EngineSchedule::Update, |mut queue: ResMut<SpawnQueue>| {
            queue.enqueue((WorldPos::new(42, 0, 0), Health(50)));
        });

        schedules.add_system(EngineSchedule::PostUpdate, flush_entity_queues);

        schedules.run(&mut world, 1.0 / 60.0);

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

        schedules.add_system(
            EngineSchedule::Update,
            move |mut queue: ResMut<DespawnQueue>| {
                queue.enqueue(target);
            },
        );

        schedules.add_system(EngineSchedule::PostUpdate, flush_entity_queues);

        schedules.run(&mut world, 1.0 / 60.0);

        assert!(world.get_entity(entity).is_err());
    }

    #[test]
    fn test_entity_ids_are_not_reused_immediately() {
        let mut world = World::new();
        let entity_a = spawn_entity(&mut world, WorldPos::default());
        despawn_entity(&mut world, entity_a);

        let entity_b = spawn_entity(&mut world, WorldPos::default());

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
        queue.enqueue(entity);

        let count = queue.flush(&mut world);
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
