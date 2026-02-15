# Physics Island Management

## Problem

Nebula Engine uses 128-bit coordinates to represent a universe-scale world, but the Rapier physics engine operates entirely in f32 local space. Simulating physics for every entity in the world is impossible — even a single planet contains billions of potential voxel colliders, and entities may be spread across multiple planets light-years apart. The engine must restrict active physics simulation to a bounded region around the player. Objects outside this region do not need rigid bodies, colliders, or collision detection — they are effectively "frozen" until the player approaches. Without this spatial partitioning, the physics engine would either run out of memory, lose precision far from the origin, or spend cycles simulating collisions nobody can observe. The challenge is making this boundary seamless: objects must gain and lose physics bodies without visible popping, teleportation, or inconsistency.

## Solution

### Physics Island Concept

A **physics island** is a spherical region of local f32 space centered on the player (or camera). Only entities and chunks within this island have active Rapier rigid bodies and colliders. The island moves with the player through the i128 world, but from Rapier's perspective, the player is always near the origin and the world moves around them.

The island is defined by:

```rust
pub struct PhysicsIsland {
    /// Center of the island in world coordinates (i128).
    pub center: WorldPos,
    /// Radius in meters. Objects within this radius get physics bodies.
    pub radius: f32,
    /// Hysteresis buffer to prevent rapid add/remove at the boundary.
    pub hysteresis: f32,
    /// Set of entities currently inside the island with active physics.
    pub active_entities: HashSet<Entity>,
    /// Set of chunk coordinates with active colliders.
    pub active_chunks: HashSet<ChunkCoord>,
}
```

Default `radius` is 512 meters. The `hysteresis` of 16 meters means an object must be within 512m to gain a body, but must be beyond 528m to lose it. This prevents rapid toggling at the boundary.

### Island Update System

Each `FixedUpdate` tick, before the physics step, a system evaluates all physics-eligible entities:

```rust
fn physics_island_update_system(
    mut island: ResMut<PhysicsIsland>,
    mut physics: ResMut<PhysicsWorld>,
    player_query: Query<&WorldPos, With<Player>>,
    entity_query: Query<(Entity, &WorldPos, Option<&RigidBodyHandle>)>,
) {
    let player_pos = player_query.single();
    island.center = *player_pos;

    for (entity, world_pos, body_handle) in entity_query.iter() {
        let distance = world_pos.distance_to(&island.center);

        if distance <= island.radius as f64 && body_handle.is_none() {
            // Entity entered the island — create a physics body
            let local_pos = world_to_local(world_pos, &island.center);
            let body = RigidBodyBuilder::dynamic()
                .translation(vector![local_pos.x, local_pos.y, local_pos.z])
                .build();
            let handle = physics.rigid_body_set.insert(body);
            // Attach handle to entity via ECS command
        }

        if distance > (island.radius + island.hysteresis) as f64
            && body_handle.is_some()
        {
            // Entity left the island — remove physics body
            let handle = body_handle.unwrap();
            physics.rigid_body_set.remove(
                handle.0,
                &mut physics.island_manager,
                &mut physics.collider_set,
                &mut physics.impulse_joint_set,
                &mut physics.multibody_joint_set,
                true,
            );
            // Remove handle component from entity
        }
    }
}
```

### Chunk Collider Management

The same island boundary applies to voxel chunk colliders. Only chunks whose center falls within the island radius get static colliders generated from their voxel data. This is coordinated with the chunk lifecycle system (story 11) — chunk load/unload events are filtered through the island check.

### Configurable Radius

The island radius is exposed as a configuration parameter:

```rust
impl PhysicsIsland {
    pub fn set_radius(&mut self, radius: f32) {
        self.radius = radius;
        self.hysteresis = (radius * 0.03).max(8.0); // 3% of radius, minimum 8m
    }
}
```

Larger radii simulate more of the world but cost more CPU. On lower-end hardware, the radius can be reduced to 256m or even 128m. The hysteresis scales proportionally.

### Entity State Preservation

When an entity leaves the island, its last known velocity, angular velocity, and position are cached in a `FrozenPhysicsState` component. When the entity re-enters the island, these values are used to reconstruct its rigid body so it resumes motion seamlessly:

```rust
pub struct FrozenPhysicsState {
    pub linear_velocity: glam::Vec3,
    pub angular_velocity: glam::Vec3,
    pub was_sleeping: bool,
}
```

### Ordering

The island update system must run **before** the i128-to-f32 bridge (story 03) and **before** the physics step. The schedule ordering is: island update -> coordinate bridge -> physics step -> bridge writeback.

## Outcome

A `PhysicsIsland` resource exists in the ECS world. Only entities and chunks within the configured radius have active Rapier rigid bodies and colliders. Moving through the world seamlessly activates and deactivates physics for nearby objects. The physics engine never processes more than a bounded number of bodies regardless of world size. `cargo test -p nebula-physics` passes all island management tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Physics islands partition the simulation. Distant objects are simulated independently. The console logs island counts for diagnostics.

## Crates & Dependencies

- `rapier3d = "0.32"` — Physics engine; bodies and colliders are added/removed as entities enter/leave the island
- `bevy_ecs = "0.18"` — ECS framework for resource management, queries, and system scheduling
- `glam = "0.32"` — Vector math for distance calculations and local-space position conversion
- `nebula-coords` (internal) — `WorldPos` type (i128-based) and coordinate conversion utilities

## Unit Tests

- **`test_object_inside_island_has_body`** — Create a `PhysicsIsland` with radius 512m centered at the origin. Place an entity at `WorldPos(100, 0, 100)`. Run the island update system. Assert the entity has a `RigidBodyHandle` component and the rigid body exists in `PhysicsWorld.rigid_body_set`.

- **`test_object_outside_island_has_no_body`** — Create an island with radius 512m. Place an entity at `WorldPos(1000, 0, 1000)` (distance ~1414m). Run the island update. Assert the entity does **not** have a `RigidBodyHandle` and `rigid_body_set` is empty.

- **`test_object_crossing_boundary_gains_body`** — Place an entity at `WorldPos(600, 0, 0)` (outside). Run update, assert no body. Move entity to `WorldPos(400, 0, 0)` (inside). Run update again, assert body exists. Verifies dynamic activation.

- **`test_object_crossing_boundary_loses_body`** — Place an entity inside the island, run update to create its body. Move the entity beyond `radius + hysteresis`. Run update. Assert the body has been removed from `rigid_body_set` and the `RigidBodyHandle` component is gone.

- **`test_island_moves_with_player`** — Set the player's `WorldPos` to `(0, 0, 0)`, run update. Move the player to `(10000, 0, 0)`, run update. Assert `island.center` matches the new player position. Entities near the old position should lose bodies; entities near the new position should gain them.

- **`test_island_radius_configurable`** — Create an island, call `set_radius(256.0)`. Assert `island.radius == 256.0`. Place an entity at 300m distance, run update, assert no body (it is outside the smaller island). Change radius to 512m, run update, assert body exists.

- **`test_hysteresis_prevents_flicker`** — Place an entity exactly at radius distance (512m). Run update — entity gains a body. Move entity to 520m (inside hysteresis band: radius < distance < radius + hysteresis). Run update — body should **not** be removed. Move to 530m (beyond hysteresis). Run update — now body is removed. Verifies the hysteresis buffer prevents rapid toggling.
