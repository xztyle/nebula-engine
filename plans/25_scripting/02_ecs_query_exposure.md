# ECS Query Exposure

## Problem

Scripts need to read and write game state to be useful, but allowing unrestricted mutable access to the ECS world from a scripting context would introduce data races, invalidate Rust's borrow-checker guarantees, and risk corrupting internal engine state. A controlled, well-defined API surface is needed that lets scripts query entities and components while keeping mutations safe through deferral.

## Solution

Register a set of Rhai functions that operate through a `ScriptContext` object passed into every script evaluation. The context holds read-only references to the ECS world for queries and a mutable command buffer for writes.

### ScriptContext

```rust
pub struct ScriptContext {
    /// Read-only snapshot of relevant world state for this frame
    pub world_snapshot: Arc<WorldSnapshot>,
    /// Deferred mutations collected during script execution
    pub commands: RefCell<Vec<ScriptCommand>>,
}
```

The `WorldSnapshot` is built at the start of the scripting system's execution by iterating the relevant archetypes once and caching positions, entity metadata, and spatial-index references. This means scripts always see a consistent view of the world within a single frame.

### Registered Functions

Each function is registered on the `rhai::Engine` via `register_fn`:

```rust
// Read position of an entity (128-bit coords downcast to f64 for script use)
fn get_position(ctx: &ScriptContext, entity_id: ScriptEntityId) -> Result<ScriptVec3, Box<EvalAltResult>>

// Queue a position update (applied after all scripts run)
fn set_position(ctx: &ScriptContext, entity_id: ScriptEntityId, pos: ScriptVec3) -> Result<(), Box<EvalAltResult>>

// Spatial query: find all entities within radius of a point
fn get_entities_near(ctx: &ScriptContext, pos: ScriptVec3, radius: f64) -> Result<Vec<Dynamic>, Box<EvalAltResult>>

// Queue entity creation, returns a provisional EntityId
fn spawn_entity(ctx: &ScriptContext, type_name: &str) -> Result<ScriptEntityId, Box<EvalAltResult>>

// Queue entity destruction
fn despawn_entity(ctx: &ScriptContext, entity_id: ScriptEntityId) -> Result<(), Box<EvalAltResult>>
```

### Deferred Mutation Pipeline

All write operations produce `ScriptCommand` variants:

```rust
pub enum ScriptCommand {
    SetPosition { entity: EntityId, position: DVec3_128 },
    SpawnEntity { archetype: String, provisional_id: u64 },
    DespawnEntity { entity: EntityId },
}
```

After all scripts for the current frame have been evaluated, the `apply_script_commands` system drains the command buffer and applies them to the real ECS world in a single exclusive-access system. This mirrors the pattern used by Bevy's `Commands` and keeps the scripting system compatible with parallel system scheduling.

### Coordinate Conversion

The engine uses 128-bit fixed-point coordinates internally. Script-facing functions convert to/from `f64` triples. The conversion uses the script's context entity as the local origin to preserve precision -- coordinates in scripts are always relative to the entity running the script. This prevents the floating-point precision issues that would occur if 128-bit coordinates were naively cast to f64 in absolute space.

## Outcome

Five ECS-interop functions registered on the Rhai engine, a deferred command buffer system that safely applies mutations after script execution, and coordinate conversion utilities that preserve precision by operating relative to the scripting entity's origin.

## Demo Integration

**Demo crate:** `nebula-demo`

Scripts can call `get_player_position()` and `count_entities()` to query the ECS world from Rhai.

## Crates & Dependencies

| Crate | Version | Purpose |
|---|---|---|
| `rhai` | `1.23` | Script function registration |
| `parking_lot` | `0.12` | Fast `RefCell`-like interior mutability for command buffer |

## Unit Tests

```rust
#[test]
fn test_get_position_returns_correct_value() {
    let (ctx, entity) = setup_test_context_with_entity_at(10.0, 20.0, 30.0);
    let result = scripted_get_position(&ctx, ScriptEntityId(entity.to_bits()));
    assert!((result.x - 10.0).abs() < f64::EPSILON);
    assert!((result.y - 20.0).abs() < f64::EPSILON);
    assert!((result.z - 30.0).abs() < f64::EPSILON);
}

#[test]
fn test_set_position_queues_update() {
    let (ctx, entity) = setup_test_context_with_entity_at(0.0, 0.0, 0.0);
    let new_pos = ScriptVec3 { x: 5.0, y: 5.0, z: 5.0 };
    scripted_set_position(&ctx, ScriptEntityId(entity.to_bits()), new_pos).unwrap();
    let commands = ctx.commands.borrow();
    assert_eq!(commands.len(), 1);
    assert!(matches!(commands[0], ScriptCommand::SetPosition { .. }));
}

#[test]
fn test_entities_near_returns_nearby() {
    let ctx = setup_test_context_with_entities_at(&[
        (1.0, 0.0, 0.0),   // within radius 5
        (10.0, 0.0, 0.0),  // within radius 15
        (100.0, 0.0, 0.0), // outside radius 15
    ]);
    let origin = ScriptVec3 { x: 0.0, y: 0.0, z: 0.0 };
    let result = scripted_get_entities_near(&ctx, origin, 15.0).unwrap();
    assert_eq!(result.len(), 2);
}

#[test]
fn test_spawn_creates_entity() {
    let ctx = setup_empty_test_context();
    let id = scripted_spawn_entity(&ctx, "npc").unwrap();
    let commands = ctx.commands.borrow();
    assert_eq!(commands.len(), 1);
    assert!(matches!(commands[0], ScriptCommand::SpawnEntity { .. }));
    assert_ne!(id.0, 0); // provisional ID should be nonzero
}

#[test]
fn test_despawn_removes_entity() {
    let (ctx, entity) = setup_test_context_with_entity_at(0.0, 0.0, 0.0);
    scripted_despawn_entity(&ctx, ScriptEntityId(entity.to_bits())).unwrap();
    let commands = ctx.commands.borrow();
    assert_eq!(commands.len(), 1);
    assert!(matches!(commands[0], ScriptCommand::DespawnEntity { .. }));
}
```
