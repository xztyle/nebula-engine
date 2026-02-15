# Physics Region Management

## Problem

A game that spans planetary surfaces, orbital space, vehicle interiors, and space stations needs fundamentally different physics rules in different locations. A player walking on a planet surface experiences 9.81 m/s^2 gravity pulling toward the planet center with normal friction. The same player floating in deep space experiences zero gravity with zero air friction. Inside a rotating space station, artificial gravity pulls outward from the rotation axis. Inside a landed spaceship, gravity matches the planet surface but the reference frame is the ship interior. The engine must track which physics "region" each entity occupies, apply the correct rules, and handle transitions between regions without jarring discontinuities. Without region management, every system that cares about gravity, friction, or damping would need to independently compute the environment, leading to inconsistency and duplicated logic.

## Solution

### PhysicsRegion Component

A `PhysicsRegion` defines the physics rules for a spatial volume:

```rust
#[derive(Component)]
pub struct PhysicsRegion {
    /// Type of physics environment.
    pub region_type: PhysicsRegionType,
    /// Gravity override. If Some, this replaces the computed gravity from sources.
    pub gravity_override: Option<GravityConfig>,
    /// Linear damping applied to entities in this region (air resistance).
    pub linear_damping: f32,
    /// Angular damping applied to entities in this region.
    pub angular_damping: f32,
    /// Friction multiplier applied to all contacts within this region.
    pub friction_multiplier: f32,
    /// Spatial bounds of this region in world coordinates.
    pub bounds: RegionBounds,
    /// Priority. Higher priority regions override lower ones when overlapping.
    pub priority: i32,
}

#[derive(Clone)]
pub enum PhysicsRegionType {
    /// Planet surface: gravity toward center, air resistance, normal friction.
    PlanetSurface {
        planet_entity: Entity,
    },
    /// Open space: no gravity, no air resistance, no friction.
    Space,
    /// Vehicle interior: gravity relative to the vehicle's orientation.
    VehicleInterior {
        vehicle_entity: Entity,
    },
    /// Space station with artificial gravity (centripetal or generated).
    ArtificialGravity {
        gravity_direction: glam::Vec3,
        gravity_magnitude: f32,
    },
    /// Underwater: high damping, reduced gravity.
    Underwater {
        fluid_density: f32,
    },
    /// Custom region with fully specified parameters.
    Custom,
}

pub struct GravityConfig {
    pub direction: glam::Vec3,
    pub magnitude: f32,
}
```

### Region Bounds

Regions occupy spatial volumes defined by various shapes:

```rust
pub enum RegionBounds {
    /// Sphere defined by center (WorldPos) and radius.
    Sphere { center: WorldPos, radius: f64 },
    /// Axis-aligned bounding box.
    Aabb { min: WorldPos, max: WorldPos },
    /// The entire space beyond a sphere (e.g., space beyond atmosphere).
    OutsideSphere { center: WorldPos, radius: f64 },
    /// Attached to an entity's collider volume (e.g., ship interior).
    EntityVolume { entity: Entity },
    /// Infinite region (fallback/default).
    Infinite,
}
```

### Region Detection System

Each tick, determine which region each entity occupies:

```rust
#[derive(Component)]
pub struct CurrentPhysicsRegion {
    pub region_entity: Option<Entity>,
    pub region_type: PhysicsRegionType,
    /// Blend factor for transitions (0.0 = fully in previous, 1.0 = fully in current).
    pub transition_blend: f32,
    /// Previous region for blending during transitions.
    pub previous_region: Option<Entity>,
}

fn region_detection_system(
    regions: Query<(Entity, &PhysicsRegion)>,
    mut entities: Query<(&WorldPos, &mut CurrentPhysicsRegion)>,
) {
    for (entity_pos, mut current) in entities.iter_mut() {
        let mut best_region: Option<(Entity, &PhysicsRegion)> = None;
        let mut best_priority = i32::MIN;

        for (region_entity, region) in regions.iter() {
            if region.bounds.contains(entity_pos) && region.priority > best_priority {
                best_region = Some((region_entity, region));
                best_priority = region.priority;
            }
        }

        if let Some((region_entity, region)) = best_region {
            if current.region_entity != Some(region_entity) {
                // Region changed — start transition.
                current.previous_region = current.region_entity;
                current.region_entity = Some(region_entity);
                current.region_type = region.region_type.clone();
                current.transition_blend = 0.0;
            }
        }
    }
}
```

### Smooth Transitions

When an entity moves between regions, physics parameters are interpolated over a short duration (default: 0.5 seconds) to prevent jarring changes:

```rust
fn region_transition_system(
    time: Res<FixedTime>,
    regions: Query<&PhysicsRegion>,
    mut entities: Query<&mut CurrentPhysicsRegion>,
    mut physics: ResMut<PhysicsWorld>,
    body_query: Query<&RigidBodyHandle>,
) {
    let transition_speed = 2.0; // 1.0 / 0.5 seconds

    for mut current in entities.iter_mut() {
        if current.transition_blend < 1.0 {
            current.transition_blend =
                (current.transition_blend + time.delta_seconds() * transition_speed)
                    .min(1.0);
        }
    }
}
```

During the transition, gravity direction, damping, and friction are linearly interpolated between the old and new region values. The `LocalGravity` system (story 07) checks for gravity overrides from the current region before falling back to computed gravity:

```rust
fn apply_region_gravity(
    current: &CurrentPhysicsRegion,
    regions: &Query<&PhysicsRegion>,
    computed_gravity: &GravityResult,
) -> GravityResult {
    if let Some(region_entity) = current.region_entity {
        if let Ok(region) = regions.get(region_entity) {
            if let Some(ref override_gravity) = region.gravity_override {
                let blend = current.transition_blend;
                return GravityResult {
                    direction: computed_gravity.direction.lerp(
                        override_gravity.direction, blend
                    ),
                    magnitude: computed_gravity.magnitude * (1.0 - blend)
                        + override_gravity.magnitude * blend,
                };
            }
        }
    }
    *computed_gravity
}
```

### Vehicle Interior Regions

Vehicle interiors are special: their region bounds move with the vehicle entity. The `EntityVolume` bounds type tracks the vehicle's collider AABB and tests whether entities are inside it. Gravity inside a landed ship matches the planet surface; gravity inside a flying ship can be zero (realistic) or ship-relative (gameplay preference):

```rust
fn update_vehicle_region_bounds_system(
    physics: Res<PhysicsWorld>,
    mut regions: Query<(&mut PhysicsRegion, &VehicleInteriorMarker)>,
    vehicles: Query<(&WorldPos, &RigidBodyHandle), With<Vehicle>>,
) {
    for (mut region, marker) in regions.iter_mut() {
        if let Ok((vehicle_pos, _)) = vehicles.get(marker.vehicle_entity) {
            if let RegionBounds::EntityVolume { .. } = &region.bounds {
                // Bounds automatically track the entity each frame.
            }
        }
    }
}
```

### Default Regions

At world initialization, two default regions are created:
1. **Space region** (priority 0, `Infinite` bounds): zero gravity, zero damping, zero friction. Covers everything by default.
2. **Planet surface regions** (priority 10, `Sphere` bounds matching the planet radius + atmosphere): surface gravity, atmospheric damping, normal friction.

Higher-priority regions (vehicle interiors at priority 20, custom zones at priority 30) override the defaults where they overlap.

## Outcome

A `PhysicsRegion` system classifies every entity into a physics environment with appropriate gravity, damping, and friction. Transitions between regions are smooth over 0.5 seconds. Vehicle interiors, space stations, and custom zones override default planetary/space physics. `cargo test -p nebula-physics` passes all region detection and transition tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Only terrain near the player has active collision shapes. Distant terrain is physics-inactive. The console logs `Active colliders: 47, total chunks: 2,300`.

## Crates & Dependencies

- `rapier3d = "0.32"` — Per-body damping and friction configuration driven by region parameters
- `bevy_ecs = "0.18"` — ECS framework for components (`PhysicsRegion`, `CurrentPhysicsRegion`), systems, queries, and entity references
- `glam = "0.32"` — Vector math for gravity direction interpolation (`lerp`) and spatial calculations
- `nebula-math` (internal) — `WorldPos` (i128) for region bounds containment checks
- `nebula-coords` (internal) — Spatial containment tests for sphere and AABB bounds

## Unit Tests

- **`test_surface_region_has_gravity`** — Create a planet surface region with `gravity_override = Some(GravityConfig { direction: Vec3::NEG_Y, magnitude: 9.81 })`. Place an entity inside the region. Run region detection. Assert `current.region_type` is `PlanetSurface` and the applied gravity magnitude is 9.81.

- **`test_space_region_has_no_gravity`** — Create only the default space region (infinite, zero gravity). Place an entity anywhere. Run detection. Assert the region type is `Space` and gravity magnitude is 0.0 and linear damping is 0.0.

- **`test_vehicle_interior_has_local_gravity`** — Create a vehicle entity with an interior region specifying `gravity_override = Some(GravityConfig { direction: Vec3::NEG_Y, magnitude: 9.81 })`. Place an entity inside the vehicle bounds. Run detection. Assert the entity's region is `VehicleInterior` with the correct gravity, even if the vehicle is floating in space (where the default region would give zero-g).

- **`test_region_transitions_are_smooth`** — Place an entity in a surface region (gravity 9.81). Move it into a space region (gravity 0.0). Run the transition system for multiple ticks. Assert that `transition_blend` increases from 0.0 toward 1.0, and the interpolated gravity magnitude smoothly decreases from 9.81 toward 0.0 over the transition duration.

- **`test_entity_detects_region_change`** — Place an entity in region A. Assert `current.region_entity == Some(A)`. Move the entity into region B. Run detection. Assert `current.region_entity == Some(B)` and `current.previous_region == Some(A)`.

- **`test_higher_priority_region_wins`** — Create an outer region (priority 10, sphere) and an inner region (priority 20, smaller sphere) overlapping inside the outer region. Place an entity in the overlap. Run detection. Assert the entity is assigned to the higher-priority inner region.
