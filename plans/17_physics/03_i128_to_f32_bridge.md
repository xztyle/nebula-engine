# i128-to-f32 Physics Bridge

## Problem

Nebula Engine represents all world positions using 128-bit integer coordinates (`WorldPos` with i128 components) to support a universe-scale world without floating-point precision loss. Rapier 0.32, like all real-time physics engines, operates exclusively in f32 space. A naive conversion from i128 to f32 would lose precision catastrophically — f32 can only represent ~7 significant decimal digits, while an i128 position like `(340_282_366_920_938, 42, -170_141_183_460_469)` has 15+ significant digits. The engine must bridge these two coordinate systems every physics tick: converting world positions to f32 local positions before stepping Rapier, then converting Rapier's results back to world positions afterward. The bridge must be precise to within 1mm at any location in the universe, must not cause objects to jitter or teleport, and must handle the case where objects drift far from the local origin (requiring a re-centering operation).

## Solution

### Local-Space Origin

The physics simulation operates in a local coordinate frame whose origin is defined by a reference point — typically the player or camera position in i128 world space. All positions passed to Rapier are **relative offsets** from this origin, expressed in f32 meters:

```rust
pub struct PhysicsOrigin {
    /// The world-space position that maps to Rapier's (0, 0, 0).
    pub world_origin: WorldPos,
}
```

An entity at `WorldPos(1_000_000_000_100, 50, 1_000_000_000_200)` with an origin at `WorldPos(1_000_000_000_000, 50, 1_000_000_000_000)` becomes Rapier position `(100.0, 0.0, 200.0)` — well within f32 precision.

### World-to-Local Conversion

Before each physics step, a system converts every entity's `WorldPos` to a Rapier body translation:

```rust
fn world_to_local(world_pos: &WorldPos, origin: &WorldPos) -> glam::Vec3 {
    // Subtraction in i128 space preserves full precision.
    let dx = (world_pos.x - origin.x) as f64;
    let dy = (world_pos.y - origin.y) as f64;
    let dz = (world_pos.z - origin.z) as f64;

    // Convert to f32 only after the subtraction.
    // Within a 512m island radius, the f64->f32 cast loses < 0.01mm.
    glam::Vec3::new(dx as f32, dy as f32, dz as f32)
}
```

The key insight: the **subtraction happens in i128 space**, producing a small offset. Only the small offset is cast to f32. This avoids the catastrophic precision loss that would occur if the large i128 value were converted to f32 directly.

The intermediate f64 step handles the i128-to-floating conversion. Within a 512m island radius, the maximum offset is 512, and f64 represents integers up to 2^53 exactly — so the i128-to-f64 cast is lossless for any offset under ~9 quadrillion meters. The f64-to-f32 cast then loses precision only beyond ~7 digits, which at 512m maximum offset means sub-millimeter accuracy.

### Local-to-World Writeback

After Rapier steps, each body's new position is read back and converted to `WorldPos`:

```rust
fn local_to_world(local_pos: &glam::Vec3, origin: &WorldPos) -> WorldPos {
    // Round the f32 offset to the nearest integer (millimeter-scale units).
    // WorldPos uses i128 with sub-meter granularity defined by WORLD_UNITS_PER_METER.
    let dx = (local_pos.x as f64 * WORLD_UNITS_PER_METER as f64).round() as i128;
    let dy = (local_pos.y as f64 * WORLD_UNITS_PER_METER as f64).round() as i128;
    let dz = (local_pos.z as f64 * WORLD_UNITS_PER_METER as f64).round() as i128;

    WorldPos {
        x: origin.x + dx,
        y: origin.y + dy,
        z: origin.z + dz,
    }
}
```

Where `WORLD_UNITS_PER_METER` is the scaling factor between engine world units and meters (e.g., 1000 for millimeter-resolution i128 coordinates, or 1 if i128 units are meters).

### Bridge Systems

Two systems bracket the physics step:

```rust
/// Runs BEFORE physics_step_system in FixedUpdate.
fn bridge_write_to_rapier(
    origin: Res<PhysicsOrigin>,
    physics: ResMut<PhysicsWorld>,
    query: Query<(&WorldPos, &RigidBodyHandle), Changed<WorldPos>>,
) {
    for (world_pos, handle) in query.iter() {
        let local = world_to_local(world_pos, &origin.world_origin);
        if let Some(body) = physics.rigid_body_set.get_mut(handle.0) {
            let translation = body.translation_mut();
            translation.x = local.x;
            translation.y = local.y;
            translation.z = local.z;
        }
    }
}

/// Runs AFTER physics_step_system in FixedUpdate.
fn bridge_read_from_rapier(
    origin: Res<PhysicsOrigin>,
    physics: Res<PhysicsWorld>,
    mut query: Query<(&mut WorldPos, &RigidBodyHandle)>,
) {
    for (mut world_pos, handle) in query.iter_mut() {
        if let Some(body) = physics.rigid_body_set.get(handle.0) {
            let t = body.translation();
            let local = glam::Vec3::new(t.x, t.y, t.z);
            *world_pos = local_to_world(&local, &origin.world_origin);
        }
    }
}
```

### Re-Centering

As the player moves, objects near the edge of the island have larger f32 offsets from the origin. When the player moves more than a threshold distance (default: 64m) from the current `PhysicsOrigin`, the origin is shifted to the player's current position. All active Rapier bodies are then adjusted:

```rust
fn recenter_physics_origin(
    mut origin: ResMut<PhysicsOrigin>,
    mut physics: ResMut<PhysicsWorld>,
    player_query: Query<&WorldPos, With<Player>>,
) {
    let player_pos = player_query.single();
    let shift = world_to_local(player_pos, &origin.world_origin);

    if shift.length() > 64.0 {
        // Update the origin.
        let old_origin = origin.world_origin;
        origin.world_origin = *player_pos;

        // Shift all Rapier body positions by the inverse offset.
        for (_, body) in physics.rigid_body_set.iter_mut() {
            let t = body.translation();
            let new_t = vector![t.x - shift.x, t.y - shift.y, t.z - shift.z];
            body.set_translation(new_t, false); // false = don't wake sleeping bodies
        }
    }
}
```

The re-centering subtracts the shift from every body. Because the shift is applied uniformly, relative distances between bodies are perfectly preserved — objects do not teleport or change their relative positions.

### Schedule Order

The full `FixedUpdate` order:

1. `physics_island_update_system` — Add/remove bodies entering/leaving the island
2. `recenter_physics_origin` — Shift the origin if the player has moved far enough
3. `bridge_write_to_rapier` — Sync world positions to Rapier
4. `physics_step_system` — Step Rapier
5. `bridge_read_from_rapier` — Sync Rapier positions back to world

## Outcome

Every physics tick, entity positions are seamlessly converted between i128 world space and f32 Rapier space. Objects anywhere in the 128-bit universe experience accurate physics with sub-millimeter precision. The bridge is invisible to gameplay code — systems read and write `WorldPos` as usual, and the bridge handles the rest. `cargo test -p nebula-physics` passes all bridge precision and roundtrip tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Physics operates in local f32 space around the camera. The i128-to-f32 bridge maps world positions each tick so the player does not jitter at large coordinates.

## Crates & Dependencies

- `rapier3d = "0.32"` — Physics engine operating in f32 local space
- `bevy_ecs = "0.18"` — ECS framework for systems, queries, resources, and change detection
- `glam = "0.32"` — f32 vector math for intermediate calculations
- `nebula-math` (internal) — `WorldPos` type with i128 components, `WORLD_UNITS_PER_METER` constant
- `nebula-coords` (internal) — Coordinate frame conversions and spatial utilities

## Unit Tests

- **`test_world_to_local_accuracy`** — Set origin to `WorldPos(1_000_000_000_000, 0, 1_000_000_000_000)`. Convert `WorldPos(1_000_000_000_100, 50, 1_000_000_000_200)` to local. Assert result is `Vec3(100.0, 50.0, 200.0)` within f32 epsilon. Verifies that large absolute coordinates produce accurate small local offsets.

- **`test_local_to_world_roundtrip`** — Set origin to an arbitrary large `WorldPos`. Convert a nearby `WorldPos` to local, then convert back to world. Assert the roundtrip result matches the original within 1mm (1 world unit if `WORLD_UNITS_PER_METER == 1000`, or exact if units are meters). Repeat for 100 random positions within 512m of the origin.

- **`test_recenter_preserves_relative_positions`** — Place two bodies at known `WorldPos` values 50m apart. Record their relative distance. Trigger a re-center (move the player 100m). After re-centering, compute the relative distance between the two bodies in Rapier space. Assert it matches the original within f32 epsilon. Verifies that re-centering does not alter inter-body distances.

- **`test_recenter_does_not_teleport_objects`** — Place a body at a known position. Record its `WorldPos`. Trigger a re-center. Run the bridge writeback. Assert the entity's `WorldPos` has not changed. Verifies that re-centering is invisible to the world-space representation.

- **`test_bridge_handles_origin_shift`** — Move the player 100m, trigger re-center. Add a new entity 10m from the player. Run the bridge write, physics step, and bridge read. Assert the new entity's `WorldPos` is consistent — it should not have been placed 100m away due to a stale origin.

- **`test_precision_valid_within_island_radius`** — For positions at 0m, 128m, 256m, and 512m from the origin, convert `WorldPos` to local f32 and back. Assert each roundtrip error is less than 1mm. Verifies that the bridge maintains precision across the full island radius.
