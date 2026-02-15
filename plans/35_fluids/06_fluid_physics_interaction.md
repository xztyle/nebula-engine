# Fluid Physics Interaction

## Problem

Entities in the game world — players, creatures, dropped items, vehicles — must physically interact with fluid. In the real world, an object in water experiences buoyancy (upward force proportional to displaced volume) and drag (resistance proportional to velocity). Without fluid-physics interaction, entities would fall through water at full speed as if it were air, items would never float, and players would have no sense of being in a liquid. The engine needs buoyancy, drag, fluid damage (lava), density-based sinking/floating, and a swim movement mode that replaces standard ground movement when the player is submerged. All of this must integrate with the Rapier physics world (Epic 17) and the cellular automaton fluid state (stories 01-02), using the planet-relative gravity direction (story 03).

## Solution

### Fluid Overlap Detection

Each physics entity with a `FluidInteraction` component checks which fluid cells it overlaps. The overlap is computed by sampling the entity's AABB against the voxel grid:

```rust
/// Component marking an entity as interacting with fluids.
#[derive(Clone, Debug)]
pub struct FluidInteraction {
    /// The entity's density in kg/m^3. Determines buoyancy.
    /// Wood ~600, Human ~1010, Iron ~7870.
    pub density: f32,
    /// Volume of the entity in cubic voxels (approximate).
    pub volume: f32,
    /// Whether this entity takes damage from damaging fluids.
    pub takes_fluid_damage: bool,
    /// Current submersion state (computed each tick).
    pub submersion: SubmersionState,
}

/// Describes how much of an entity is submerged in fluid.
#[derive(Clone, Debug, Default)]
pub struct SubmersionState {
    /// Fraction of the entity's volume that is submerged (0.0 to 1.0).
    pub submerged_fraction: f32,
    /// The fluid type the entity is most submerged in (if any).
    pub primary_fluid: Option<FluidTypeId>,
    /// The average fluid level across overlapping cells.
    pub average_level: f32,
    /// The deepest point of submersion (distance below fluid surface).
    pub depth: f32,
}
```

### Submersion Calculation

Each tick, the system samples the fluid cells overlapping the entity's bounding box:

```rust
/// Compute the submersion state for an entity given its AABB and the fluid grid.
pub fn compute_submersion(
    entity_aabb: &Aabb,
    chunk_data: &ChunkFluidData,
    chunk_origin: &WorldPosition,
) -> SubmersionState {
    let mut total_cells = 0u32;
    let mut submerged_cells = 0u32;
    let mut level_sum = 0u32;
    let mut fluid_type_counts: HashMap<FluidTypeId, u32> = HashMap::new();
    let mut max_depth: f32 = 0.0;

    // Iterate voxels overlapping the AABB
    let min = entity_aabb.min.floor();
    let max = entity_aabb.max.ceil();
    for x in min.x as i32..=max.x as i32 {
        for y in min.y as i32..=max.y as i32 {
            for z in min.z as i32..=max.z as i32 {
                total_cells += 1;
                let local = to_local(x, y, z, chunk_origin);
                if let Some(state) = chunk_data.get(local) {
                    if !state.is_empty() {
                        submerged_cells += 1;
                        level_sum += state.level as u32;
                        *fluid_type_counts.entry(state.fluid_type).or_insert(0) += 1;
                        let surface_y = y as f32 + surface_height(state.level);
                        let entity_bottom = entity_aabb.min.y;
                        let depth = surface_y - entity_bottom;
                        if depth > max_depth { max_depth = depth; }
                    }
                }
            }
        }
    }

    if total_cells == 0 {
        return SubmersionState::default();
    }

    let primary_fluid = fluid_type_counts.into_iter()
        .max_by_key(|&(_, count)| count)
        .map(|(id, _)| id);

    SubmersionState {
        submerged_fraction: submerged_cells as f32 / total_cells as f32,
        primary_fluid,
        average_level: if submerged_cells > 0 {
            level_sum as f32 / submerged_cells as f32
        } else {
            0.0
        },
        depth: max_depth.max(0.0),
    }
}
```

### Buoyancy Force

Buoyancy follows Archimedes' principle: the upward force equals the weight of the displaced fluid. In the engine, the force is applied along the local "up" direction (opposite gravity):

```rust
/// Compute the buoyancy force vector for a submerged entity.
/// Returns force in Newtons, directed opposite to local gravity.
pub fn compute_buoyancy(
    entity: &FluidInteraction,
    submersion: &SubmersionState,
    fluid_registry: &FluidTypeRegistry,
    gravity_dir: [f32; 3],
    gravity_magnitude: f32,
) -> [f32; 3] {
    let fluid_density = match submersion.primary_fluid {
        Some(id) => fluid_registry.get(id).density,
        None => return [0.0; 3],
    };

    // Displaced volume = entity volume * submerged fraction
    let displaced_volume = entity.volume * submersion.submerged_fraction;

    // Buoyancy force magnitude = fluid_density * displaced_volume * gravity
    let force_magnitude = fluid_density * displaced_volume * gravity_magnitude;

    // Direction is opposite to gravity (upward)
    [
        -gravity_dir[0] * force_magnitude,
        -gravity_dir[1] * force_magnitude,
        -gravity_dir[2] * force_magnitude,
    ]
}
```

An entity floats when its density is less than the fluid's density (buoyancy > weight). It sinks when its density exceeds the fluid's density.

### Drag Force

Drag resists motion through the fluid, proportional to velocity and fluid viscosity:

```rust
/// Compute the drag force opposing the entity's motion through fluid.
/// Simplified drag model: F_drag = -viscosity * drag_coefficient * velocity
pub fn compute_drag(
    velocity: [f32; 3],
    viscosity: f32,
    submerged_fraction: f32,
) -> [f32; 3] {
    let drag_coeff = 0.5; // Base drag coefficient
    let factor = -viscosity * drag_coeff * submerged_fraction;
    [
        velocity[0] * factor,
        velocity[1] * factor,
        velocity[2] * factor,
    ]
}
```

Water (viscosity 1.0) provides moderate drag. Lava (viscosity 50.0) makes movement extremely slow, trapping entities. Oil (viscosity 5.0) is in between.

### Lava Damage

When an entity is submerged in a fluid with `damage_per_second > 0`, damage is applied each tick:

```rust
/// Apply fluid damage to an entity if it is submerged in a damaging fluid.
pub fn compute_fluid_damage(
    submersion: &SubmersionState,
    fluid_registry: &FluidTypeRegistry,
    dt: f32,
) -> f32 {
    match submersion.primary_fluid {
        Some(id) => {
            let def = fluid_registry.get(id);
            def.damage_per_second * submersion.submerged_fraction * dt
        }
        None => 0.0,
    }
}
```

### Swimming Mode

When the player's submersion fraction exceeds a threshold, the movement system switches to swim mode:

```rust
/// Movement mode for entities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovementMode {
    Walking,
    Swimming,
    Flying,
}

/// Determine the movement mode based on submersion.
pub fn movement_mode_from_submersion(submersion: &SubmersionState) -> MovementMode {
    if submersion.submerged_fraction > 0.5 {
        MovementMode::Swimming
    } else {
        MovementMode::Walking
    }
}
```

Swimming mode changes:
- Gravity is reduced (partially countered by buoyancy).
- Movement input applies force in the camera look direction (up/down + forward/back).
- Jump becomes "swim up" (applies upward impulse).
- Movement speed is reduced by fluid drag.

### ECS System

```rust
fn fluid_physics_system(
    mut entities: Query<(
        &mut FluidInteraction,
        &Transform,
        &mut RigidBodyHandle,
        Option<&mut Health>,
    )>,
    physics: ResMut<PhysicsWorld>,
    chunks: Query<(&ChunkAddress, &ChunkFluidData)>,
    fluid_registry: Res<Arc<FluidTypeRegistry>>,
    gravity: Res<GravityConfig>,
    time: Res<FixedTimestep>,
) {
    for (mut interaction, transform, mut rb_handle, health) in entities.iter_mut() {
        // 1. Compute submersion
        let aabb = compute_entity_aabb(transform, interaction.volume);
        interaction.submersion = compute_submersion(&aabb, /* chunk lookup */);

        if interaction.submersion.submerged_fraction > 0.0 {
            let gravity_dir = gravity.direction_at(transform.position);
            let gravity_mag = gravity.magnitude();

            // 2. Apply buoyancy
            let buoyancy = compute_buoyancy(
                &interaction,
                &interaction.submersion,
                &fluid_registry,
                gravity_dir,
                gravity_mag,
            );
            physics.apply_force(rb_handle, buoyancy);

            // 3. Apply drag
            let velocity = physics.get_velocity(rb_handle);
            let viscosity = match interaction.submersion.primary_fluid {
                Some(id) => fluid_registry.get(id).viscosity,
                None => 1.0,
            };
            let drag = compute_drag(velocity, viscosity, interaction.submersion.submerged_fraction);
            physics.apply_force(rb_handle, drag);

            // 4. Apply damage
            if interaction.takes_fluid_damage {
                let damage = compute_fluid_damage(
                    &interaction.submersion,
                    &fluid_registry,
                    time.dt(),
                );
                if damage > 0.0 {
                    if let Some(ref mut hp) = health {
                        hp.current -= damage;
                    }
                }
            }
        }
    }
}
```

### Rapier Integration

Forces are applied to Rapier rigid bodies using `RigidBodySet::get_mut(handle).add_force(force, true)`. The buoyancy and drag forces are computed in the engine's local coordinate frame and applied directly. Since Rapier operates in f32 local space (Epic 17, story 01), no 128-bit conversion is needed for the force vectors.

## Outcome

The `nebula-fluid` crate exports `FluidInteraction`, `SubmersionState`, `MovementMode`, `compute_buoyancy`, `compute_drag`, `compute_fluid_damage`, and `fluid_physics_system`. Entities experience buoyancy, drag, and fluid damage. Light objects float; heavy objects sink. Players switch to swim mode when submerged. Lava deals damage and has extreme drag. Running `cargo test -p nebula-fluid` passes all fluid physics interaction tests. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Walking into water slows the player. Swimming uses different controls. Buoyancy keeps the player at the surface. Diving underwater applies a blue visual tint.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `rapier3d` | `0.32` | Physics engine for applying forces to rigid bodies and reading velocities |
| `bevy_ecs` | `0.18` | ECS framework for systems, queries, resources, and `FixedUpdate` schedule |
| `glam` | `0.32` | Vector math for force calculations and AABB computation |
| `hashbrown` | `0.15` | Fast `HashMap` for fluid type counting during submersion calculation |

Depends on Epic 17 (Rapier physics world), story 01 (`FluidTypeId`, `FluidTypeDef`, `FluidState`), story 03 (gravity direction), and Epic 16 (player movement).

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn water_registry() -> FluidTypeRegistry {
        let mut reg = FluidTypeRegistry::new();
        reg.register(FluidTypeDef {
            name: "water".to_string(),
            viscosity: 1.0,
            color: [0.2, 0.4, 0.9, 0.6],
            flow_speed: 1.0,
            density: 1000.0,
            damage_per_second: 0.0,
            light_emission: 0,
        }).unwrap();
        reg.register(FluidTypeDef {
            name: "lava".to_string(),
            viscosity: 50.0,
            color: [1.0, 0.3, 0.0, 0.95],
            flow_speed: 0.1,
            density: 3100.0,
            damage_per_second: 20.0,
            light_emission: 14,
        }).unwrap();
        reg
    }

    #[test]
    fn test_light_object_floats() {
        let registry = water_registry();
        let water_id = FluidTypeId(0);

        // Wood: density 600, less than water's 1000
        let entity = FluidInteraction {
            density: 600.0,
            volume: 1.0,
            takes_fluid_damage: false,
            submersion: SubmersionState {
                submerged_fraction: 1.0,
                primary_fluid: Some(water_id),
                average_level: 7.0,
                depth: 1.0,
            },
        };

        let gravity_dir = [0.0, -1.0, 0.0];
        let gravity_mag = 9.81;

        let buoyancy = compute_buoyancy(&entity, &entity.submersion, &registry, gravity_dir, gravity_mag);
        let weight = entity.density * entity.volume * gravity_mag;

        // Buoyancy should be upward (+Y)
        assert!(buoyancy[1] > 0.0, "Buoyancy must point upward");

        // Buoyancy magnitude should exceed weight for a light object
        let buoyancy_mag = (buoyancy[0].powi(2) + buoyancy[1].powi(2) + buoyancy[2].powi(2)).sqrt();
        assert!(
            buoyancy_mag > weight,
            "Buoyancy ({buoyancy_mag}) must exceed weight ({weight}) for floating object"
        );
    }

    #[test]
    fn test_heavy_object_sinks() {
        let registry = water_registry();
        let water_id = FluidTypeId(0);

        // Iron: density 7870, much more than water's 1000
        let entity = FluidInteraction {
            density: 7870.0,
            volume: 1.0,
            takes_fluid_damage: false,
            submersion: SubmersionState {
                submerged_fraction: 1.0,
                primary_fluid: Some(water_id),
                average_level: 7.0,
                depth: 1.0,
            },
        };

        let gravity_dir = [0.0, -1.0, 0.0];
        let gravity_mag = 9.81;

        let buoyancy = compute_buoyancy(&entity, &entity.submersion, &registry, gravity_dir, gravity_mag);
        let weight = entity.density * entity.volume * gravity_mag;

        let buoyancy_mag = (buoyancy[0].powi(2) + buoyancy[1].powi(2) + buoyancy[2].powi(2)).sqrt();
        assert!(
            buoyancy_mag < weight,
            "Buoyancy ({buoyancy_mag}) must be less than weight ({weight}) for sinking object"
        );
    }

    #[test]
    fn test_buoyancy_proportional_to_submerged_volume() {
        let registry = water_registry();
        let water_id = FluidTypeId(0);
        let gravity_dir = [0.0, -1.0, 0.0];
        let gravity_mag = 9.81;

        let half_submerged = SubmersionState {
            submerged_fraction: 0.5,
            primary_fluid: Some(water_id),
            average_level: 4.0,
            depth: 0.5,
        };

        let fully_submerged = SubmersionState {
            submerged_fraction: 1.0,
            primary_fluid: Some(water_id),
            average_level: 7.0,
            depth: 1.0,
        };

        let entity = FluidInteraction {
            density: 800.0,
            volume: 1.0,
            takes_fluid_damage: false,
            submersion: half_submerged.clone(),
        };

        let buoyancy_half = compute_buoyancy(&entity, &half_submerged, &registry, gravity_dir, gravity_mag);
        let buoyancy_full = compute_buoyancy(&entity, &fully_submerged, &registry, gravity_dir, gravity_mag);

        let mag_half = buoyancy_half[1];
        let mag_full = buoyancy_full[1];

        let ratio = mag_full / mag_half;
        assert!(
            (ratio - 2.0).abs() < 0.01,
            "Fully submerged buoyancy should be 2x half submerged, got ratio {ratio}"
        );
    }

    #[test]
    fn test_drag_slows_movement() {
        let velocity = [10.0, 0.0, 5.0];
        let viscosity = 1.0; // Water
        let submerged = 1.0;

        let drag = compute_drag(velocity, viscosity, submerged);

        // Drag should oppose velocity direction
        assert!(drag[0] < 0.0, "Drag X should oppose positive velocity X");
        assert_eq!(drag[1], 0.0, "Drag Y should be zero for zero velocity Y");
        assert!(drag[2] < 0.0, "Drag Z should oppose positive velocity Z");

        // Higher viscosity = more drag
        let drag_oil = compute_drag(velocity, 5.0, submerged);
        assert!(
            drag_oil[0].abs() > drag[0].abs(),
            "Oil drag should be stronger than water drag"
        );
    }

    #[test]
    fn test_lava_applies_damage() {
        let registry = water_registry();
        let lava_id = FluidTypeId(1);

        let submersion = SubmersionState {
            submerged_fraction: 1.0,
            primary_fluid: Some(lava_id),
            average_level: 7.0,
            depth: 1.0,
        };

        let dt = 1.0 / 60.0; // One physics tick
        let damage = compute_fluid_damage(&submersion, &registry, dt);

        assert!(damage > 0.0, "Lava should deal damage");

        // Expected: 20.0 dps * 1.0 submersion * (1/60)s = 0.333...
        let expected = 20.0 * 1.0 * dt;
        assert!(
            (damage - expected).abs() < 0.001,
            "Lava damage should be {expected}, got {damage}"
        );
    }

    #[test]
    fn test_water_deals_no_damage() {
        let registry = water_registry();
        let water_id = FluidTypeId(0);

        let submersion = SubmersionState {
            submerged_fraction: 1.0,
            primary_fluid: Some(water_id),
            average_level: 7.0,
            depth: 1.0,
        };

        let damage = compute_fluid_damage(&submersion, &registry, 1.0 / 60.0);
        assert_eq!(damage, 0.0, "Water should deal zero damage");
    }

    #[test]
    fn test_swimming_mode_activates_in_water() {
        let shallow = SubmersionState {
            submerged_fraction: 0.3,
            primary_fluid: Some(FluidTypeId(0)),
            average_level: 3.0,
            depth: 0.3,
        };
        assert_eq!(
            movement_mode_from_submersion(&shallow),
            MovementMode::Walking,
            "Shallow submersion should keep walking mode"
        );

        let deep = SubmersionState {
            submerged_fraction: 0.8,
            primary_fluid: Some(FluidTypeId(0)),
            average_level: 6.0,
            depth: 1.5,
        };
        assert_eq!(
            movement_mode_from_submersion(&deep),
            MovementMode::Swimming,
            "Deep submersion should activate swimming mode"
        );
    }

    #[test]
    fn test_no_buoyancy_when_not_submerged() {
        let registry = water_registry();
        let submersion = SubmersionState::default(); // Not submerged at all

        let entity = FluidInteraction {
            density: 1000.0,
            volume: 1.0,
            takes_fluid_damage: false,
            submersion: submersion.clone(),
        };

        let buoyancy = compute_buoyancy(
            &entity,
            &submersion,
            &registry,
            [0.0, -1.0, 0.0],
            9.81,
        );
        assert_eq!(buoyancy, [0.0, 0.0, 0.0], "No buoyancy when not submerged");
    }
}
```
