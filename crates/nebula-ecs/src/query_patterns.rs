//! Common ECS query patterns for the Nebula Engine.
//!
//! Provides documented, tested query patterns and helper functions that serve
//! as both reusable utilities and onboarding reference for contributors.
//! Eight canonical patterns cover movement, rendering, initialization, optional
//! components, entity lookup, exclusion, change-filtered, and multi-mutation.

use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryFilter;
use nebula_math::Vec3I128;

use crate::components::{Velocity, WorldPos};

// ---------------------------------------------------------------------------
// Additional components used by query patterns
// ---------------------------------------------------------------------------

/// Marker component for entities that should participate in simulation.
/// Added/removed rather than toggling a bool, enabling archetype-level filtering.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Simulated;

/// Opaque handle to a GPU mesh resource.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeshHandle(pub u64);

/// Temporary knockback impulse applied to an entity's velocity.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnockbackEffect {
    /// Direction and magnitude of the knockback in millimetre units.
    pub direction: Vec3I128,
}

// ---------------------------------------------------------------------------
// Pattern 1: Movement Query
// ---------------------------------------------------------------------------

/// Query for all entities that can move: have a [`WorldPos`] and [`Velocity`].
pub type MovementQuery<'w, 's> = Query<'w, 's, (&'static mut WorldPos, &'static Velocity)>;

/// Integrates velocity into position for all movable entities.
pub fn movement_system(mut query: MovementQuery) {
    for (mut pos, vel) in &mut query {
        pos.0.x += vel.0.x;
        pos.0.y += vel.0.y;
        pos.0.z += vel.0.z;
    }
}

// ---------------------------------------------------------------------------
// Helper Functions
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Active, LocalPos, Name, Scale};

    #[test]
    fn test_movement_query_matches_entities_with_both_components() {
        let mut world = World::new();
        world.spawn((WorldPos::new(0, 0, 0), Velocity::new(1, 0, 0)));
        world.spawn(WorldPos::new(5, 5, 5));
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
        world.spawn((LocalPos::new(1.0, 2.0, 3.0), MeshHandle(42), Active(true)));
        world.spawn((LocalPos::new(4.0, 5.0, 6.0), Active(true)));
        world.spawn((LocalPos::new(7.0, 8.0, 9.0), MeshHandle(99), Active(false)));

        let mut query = world.query::<(&LocalPos, &MeshHandle)>();
        let results: Vec<_> = query.iter(&world).collect();
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
            |mut count: ResMut<AddedCount>, query: Query<Entity, Added<WorldPos>>| {
                count.0 += query.iter().count() as u32;
            },
        );

        schedule.run(&mut world);
        assert_eq!(world.resource::<AddedCount>().0, 1);

        schedule.run(&mut world);
        assert_eq!(world.resource::<AddedCount>().0, 1);

        schedule.run(&mut world);
        assert_eq!(world.resource::<AddedCount>().0, 1);
    }

    #[test]
    fn test_query_with_option_handles_missing_components() {
        let mut world = World::new();
        world.spawn((LocalPos::new(1.0, 0.0, 0.0), Name::new("named")));
        world.spawn(LocalPos::new(2.0, 0.0, 0.0));

        let mut query = world.query::<(&LocalPos, Option<&Name>)>();
        let results: Vec<_> = query.iter(&world).collect();

        assert_eq!(results.len(), 2);
        let named: Vec<_> = results.iter().filter(|(_, n)| n.is_some()).collect();
        let unnamed: Vec<_> = results.iter().filter(|(_, n)| n.is_none()).collect();
        assert_eq!(named.len(), 1);
        assert_eq!(unnamed.len(), 1);
        assert_eq!(named[0].1.unwrap().0, "named");
    }

    #[test]
    fn test_exclusion_filter_without() {
        let mut world = World::new();
        world.spawn((WorldPos::new(1, 0, 0), LocalPos::default()));
        world.spawn(WorldPos::new(2, 0, 0));
        world.spawn(WorldPos::new(3, 0, 0));

        let mut query = world.query_filtered::<&WorldPos, (With<WorldPos>, Without<LocalPos>)>();
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
            .spawn((WorldPos::new(100, 200, 300), Velocity::new(10, -5, 20)))
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
        world.spawn(WorldPos::default());

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

        let entity = world.spawn((Scale(1.0), WorldPos::default())).id();
        world.spawn(Scale(2.0));

        let mut schedule = Schedule::default();
        schedule.add_systems(
            |mut count: ResMut<ChangeCount>,
             query: Query<&Scale, (Changed<Scale>, With<WorldPos>)>| {
                count.0 += query.iter().count() as u32;
            },
        );

        schedule.run(&mut world);
        assert_eq!(world.resource::<ChangeCount>().0, 1);

        schedule.run(&mut world);
        assert_eq!(world.resource::<ChangeCount>().0, 1);

        world.get_mut::<Scale>(entity).unwrap().0 = 3.0;
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
