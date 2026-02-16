//! Unit tests for physics region management.

use bevy_ecs::prelude::*;
use glam::Vec3;
use nebula_math::WorldPosition;

use crate::physics_island::IslandWorldPos;
use crate::physics_region::{
    CurrentPhysicsRegion, GravityConfig, PhysicsRegion, PhysicsRegionType, RegionBounds,
    TRANSITION_SPEED, create_default_space_region, region_detection_system,
};

fn wp(x: i128, y: i128, z: i128) -> IslandWorldPos {
    IslandWorldPos(WorldPosition::new(x, y, z))
}

#[test]
fn test_surface_region_has_gravity() {
    let mut world = World::new();
    let planet = world.spawn_empty().id();
    let region_entity = world
        .spawn(PhysicsRegion {
            region_type: PhysicsRegionType::PlanetSurface {
                planet_entity: planet,
            },
            gravity_override: Some(GravityConfig {
                direction: Vec3::NEG_Y,
                magnitude: 9.81,
            }),
            linear_damping: 0.5,
            angular_damping: 0.5,
            friction_multiplier: 1.0,
            bounds: RegionBounds::Sphere {
                center: WorldPosition::new(0, 0, 0),
                radius: 1_000_000.0,
            },
            priority: 10,
        })
        .id();

    world.spawn((wp(0, 0, 0), CurrentPhysicsRegion::default()));

    let mut schedule = Schedule::default();
    schedule.add_systems(region_detection_system);
    schedule.run(&mut world);

    let mut query = world.query::<&CurrentPhysicsRegion>();
    let current = query.iter(&world).next().unwrap();
    assert_eq!(current.region_entity, Some(region_entity));
    assert!(matches!(
        current.region_type,
        PhysicsRegionType::PlanetSurface { .. }
    ));

    let region = world.get::<PhysicsRegion>(region_entity).unwrap();
    let grav = region.gravity_override.as_ref().unwrap();
    assert!((grav.magnitude - 9.81).abs() < f32::EPSILON);
}

#[test]
fn test_space_region_has_no_gravity() {
    let mut world = World::new();
    world.spawn(create_default_space_region());
    world.spawn((wp(999, 999, 999), CurrentPhysicsRegion::default()));

    let mut schedule = Schedule::default();
    schedule.add_systems(region_detection_system);
    schedule.run(&mut world);

    let mut query = world.query::<&CurrentPhysicsRegion>();
    let current = query.iter(&world).next().unwrap();
    assert!(matches!(current.region_type, PhysicsRegionType::Space));

    let mut rq = world.query::<&PhysicsRegion>();
    let region = rq.iter(&world).next().unwrap();
    let grav = region.gravity_override.as_ref().unwrap();
    assert!((grav.magnitude).abs() < f32::EPSILON);
    assert!((region.linear_damping).abs() < f32::EPSILON);
}

#[test]
fn test_vehicle_interior_has_local_gravity() {
    let mut world = World::new();
    world.spawn(create_default_space_region());

    let vehicle = world.spawn_empty().id();
    let vehicle_region = world
        .spawn(PhysicsRegion {
            region_type: PhysicsRegionType::VehicleInterior {
                vehicle_entity: vehicle,
            },
            gravity_override: Some(GravityConfig {
                direction: Vec3::NEG_Y,
                magnitude: 9.81,
            }),
            linear_damping: 0.3,
            angular_damping: 0.3,
            friction_multiplier: 1.0,
            bounds: RegionBounds::Aabb {
                min: WorldPosition::new(-5000, -5000, -5000),
                max: WorldPosition::new(5000, 5000, 5000),
            },
            priority: 20,
        })
        .id();

    world.spawn((wp(0, 0, 0), CurrentPhysicsRegion::default()));

    let mut schedule = Schedule::default();
    schedule.add_systems(region_detection_system);
    schedule.run(&mut world);

    let mut query = world.query::<&CurrentPhysicsRegion>();
    let current = query.iter(&world).next().unwrap();
    assert_eq!(current.region_entity, Some(vehicle_region));
    assert!(matches!(
        current.region_type,
        PhysicsRegionType::VehicleInterior { .. }
    ));

    let region = world.get::<PhysicsRegion>(vehicle_region).unwrap();
    let grav = region.gravity_override.as_ref().unwrap();
    assert!((grav.magnitude - 9.81).abs() < f32::EPSILON);
}

#[test]
fn test_region_transitions_are_smooth() {
    let mut world = World::new();
    let planet = world.spawn_empty().id();
    let surface_region = world
        .spawn(PhysicsRegion {
            region_type: PhysicsRegionType::PlanetSurface {
                planet_entity: planet,
            },
            gravity_override: Some(GravityConfig {
                direction: Vec3::NEG_Y,
                magnitude: 9.81,
            }),
            linear_damping: 0.5,
            angular_damping: 0.5,
            friction_multiplier: 1.0,
            bounds: RegionBounds::Sphere {
                center: WorldPosition::new(0, 0, 0),
                radius: 100_000.0,
            },
            priority: 10,
        })
        .id();

    let space_region = world
        .spawn(PhysicsRegion {
            region_type: PhysicsRegionType::Space,
            gravity_override: Some(GravityConfig {
                direction: Vec3::ZERO,
                magnitude: 0.0,
            }),
            linear_damping: 0.0,
            angular_damping: 0.0,
            friction_multiplier: 0.0,
            bounds: RegionBounds::OutsideSphere {
                center: WorldPosition::new(0, 0, 0),
                radius: 100_000.0,
            },
            priority: 5,
        })
        .id();

    let entity = world
        .spawn((wp(0, 0, 0), CurrentPhysicsRegion::default()))
        .id();

    let mut detect = Schedule::default();
    detect.add_systems(region_detection_system);
    detect.run(&mut world);

    assert_eq!(
        world
            .get::<CurrentPhysicsRegion>(entity)
            .unwrap()
            .region_entity,
        Some(surface_region)
    );

    // Move outside sphere
    *world.get_mut::<IslandWorldPos>(entity).unwrap() = wp(200_000, 0, 0);
    detect.run(&mut world);

    {
        let c = world.get::<CurrentPhysicsRegion>(entity).unwrap();
        assert_eq!(c.region_entity, Some(space_region));
        assert!((c.transition_blend).abs() < f32::EPSILON);
        assert_eq!(c.previous_region, Some(surface_region));
    }

    // Simulate 15 ticks (0.25s) of transition
    let dt = 1.0 / 60.0;
    for _ in 0..15 {
        let mut c = world.get_mut::<CurrentPhysicsRegion>(entity).unwrap();
        if c.transition_blend < 1.0 {
            c.transition_blend = (c.transition_blend + dt * TRANSITION_SPEED).min(1.0);
        }
    }

    {
        let c = world.get::<CurrentPhysicsRegion>(entity).unwrap();
        assert!(c.transition_blend > 0.0);
        assert!(c.transition_blend < 1.0);
        let blend = c.transition_blend;
        let interp_mag = 9.81 * (1.0 - blend);
        assert!(interp_mag > 0.0);
        assert!(interp_mag < 9.81);
    }

    // Complete transition
    for _ in 0..30 {
        let mut c = world.get_mut::<CurrentPhysicsRegion>(entity).unwrap();
        if c.transition_blend < 1.0 {
            c.transition_blend = (c.transition_blend + dt * TRANSITION_SPEED).min(1.0);
        }
    }

    let c = world.get::<CurrentPhysicsRegion>(entity).unwrap();
    assert!((c.transition_blend - 1.0).abs() < f32::EPSILON);
}

#[test]
fn test_entity_detects_region_change() {
    let mut world = World::new();

    let region_a = world
        .spawn(PhysicsRegion {
            region_type: PhysicsRegionType::Space,
            gravity_override: None,
            linear_damping: 0.0,
            angular_damping: 0.0,
            friction_multiplier: 0.0,
            bounds: RegionBounds::Sphere {
                center: WorldPosition::new(0, 0, 0),
                radius: 50_000.0,
            },
            priority: 5,
        })
        .id();

    let region_b = world
        .spawn(PhysicsRegion {
            region_type: PhysicsRegionType::Custom,
            gravity_override: None,
            linear_damping: 1.0,
            angular_damping: 1.0,
            friction_multiplier: 2.0,
            bounds: RegionBounds::Sphere {
                center: WorldPosition::new(200_000, 0, 0),
                radius: 50_000.0,
            },
            priority: 5,
        })
        .id();

    let entity = world
        .spawn((wp(0, 0, 0), CurrentPhysicsRegion::default()))
        .id();

    let mut schedule = Schedule::default();
    schedule.add_systems(region_detection_system);
    schedule.run(&mut world);

    assert_eq!(
        world
            .get::<CurrentPhysicsRegion>(entity)
            .unwrap()
            .region_entity,
        Some(region_a)
    );

    *world.get_mut::<IslandWorldPos>(entity).unwrap() = wp(200_000, 0, 0);
    schedule.run(&mut world);

    let c = world.get::<CurrentPhysicsRegion>(entity).unwrap();
    assert_eq!(c.region_entity, Some(region_b));
    assert_eq!(c.previous_region, Some(region_a));
}

#[test]
fn test_higher_priority_region_wins() {
    let mut world = World::new();

    let _outer = world
        .spawn(PhysicsRegion {
            region_type: PhysicsRegionType::Space,
            gravity_override: None,
            linear_damping: 0.0,
            angular_damping: 0.0,
            friction_multiplier: 0.0,
            bounds: RegionBounds::Sphere {
                center: WorldPosition::new(0, 0, 0),
                radius: 100_000.0,
            },
            priority: 10,
        })
        .id();

    let inner = world
        .spawn(PhysicsRegion {
            region_type: PhysicsRegionType::Custom,
            gravity_override: None,
            linear_damping: 2.0,
            angular_damping: 2.0,
            friction_multiplier: 3.0,
            bounds: RegionBounds::Sphere {
                center: WorldPosition::new(0, 0, 0),
                radius: 10_000.0,
            },
            priority: 20,
        })
        .id();

    world.spawn((wp(0, 0, 0), CurrentPhysicsRegion::default()));

    let mut schedule = Schedule::default();
    schedule.add_systems(region_detection_system);
    schedule.run(&mut world);

    let mut query = world.query::<&CurrentPhysicsRegion>();
    let current = query.iter(&world).next().unwrap();
    assert_eq!(current.region_entity, Some(inner));
}
