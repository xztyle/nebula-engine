# Script Lifecycle

## Problem

Scripts need a predictable execution model tied to the game's entity lifecycle. Without well-defined lifecycle hooks, script authors would have to manually track initialization state, poll for removal, and handle frame-to-frame persistence themselves. This leads to brittle, error-prone scripts. The engine must provide clear hooks for spawn, update, despawn, and interaction, and must manage per-entity script state across frames.

## Solution

### ScriptComponent

Scripts attach to entities via an ECS component:

```rust
#[derive(Component)]
pub struct ScriptComponent {
    /// Path to the .rhai script file (key into ScriptEngine::scripts)
    pub source: String,
    /// Persistent state scope for this script instance
    pub state: ScriptState,
    /// Tracks which lifecycle phase the script is in
    pub phase: ScriptPhase,
}

pub struct ScriptState {
    /// Rhai Scope containing variables that persist between calls
    pub scope: rhai::Scope<'static>,
    /// Whether on_spawn has been called
    pub initialized: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptPhase {
    PendingSpawn,
    Active,
    PendingDespawn,
}
```

### Lifecycle Hooks

Scripts define the following functions (all optional):

| Hook | Signature (Rhai) | When Called |
|---|---|---|
| `on_spawn` | `fn on_spawn()` | Once, the first frame the component exists |
| `on_update` | `fn on_update(dt)` | Every frame while the entity is active |
| `on_despawn` | `fn on_despawn()` | Once, when the entity or component is about to be removed |
| `on_interact` | `fn on_interact(player)` | When a player interacts with the entity |

### Script Execution System

The `run_scripts` system runs in the `Update` schedule and processes entities with `ScriptComponent`:

```rust
fn run_scripts(
    mut query: Query<(Entity, &mut ScriptComponent)>,
    engine: Res<ScriptEngine>,
    time: Res<Time>,
) {
    for (entity, mut script) in query.iter_mut() {
        let ast = match engine.scripts.get(&script.source) {
            Some(ast) => ast,
            None => continue, // script not found, skip silently
        };

        let ctx = build_script_context(entity, &world_snapshot);

        // Phase: PendingSpawn -> call on_spawn, transition to Active
        if script.phase == ScriptPhase::PendingSpawn {
            if ast.has_function("on_spawn") {
                engine.call_fn_with_timeout(
                    &mut script.state.scope, ast, "on_spawn", ()
                );
            }
            script.phase = ScriptPhase::Active;
            script.state.initialized = true;
        }

        // Phase: Active -> call on_update each frame
        if script.phase == ScriptPhase::Active {
            if ast.has_function("on_update") {
                let dt = time.delta_seconds_f64();
                engine.call_fn_with_timeout(
                    &mut script.state.scope, ast, "on_update", (dt,)
                );
            }
        }

        // Phase: PendingDespawn -> call on_despawn, then remove component
        if script.phase == ScriptPhase::PendingDespawn {
            if ast.has_function("on_despawn") {
                engine.call_fn_with_timeout(
                    &mut script.state.scope, ast, "on_despawn", ()
                );
            }
        }
    }
}
```

### State Persistence

The `rhai::Scope` inside `ScriptState` persists between calls. Variables set during `on_spawn` are available in subsequent `on_update` calls. For example:

```rhai
fn on_spawn() {
    let counter = 0;       // this persists in the scope
    let health = 100.0;
}

fn on_update(dt) {
    counter += 1;           // still accessible
    if health <= 0.0 {
        // handle death
    }
}
```

### Missing Hook Handling

When a script AST does not contain a requested function, the call is silently skipped. This is checked via `ast.iter_functions().any(|f| f.name == hook_name)` before calling, so there is no runtime error for scripts that only implement a subset of hooks.

### Interaction Hook

The `on_interact` hook is triggered by the input system when a player activates an entity (e.g., pressing E near an NPC). The player's entity ID is passed as a `ScriptEntityId` argument:

```rhai
fn on_interact(player) {
    let dist = get_position(player);
    // open dialog, trigger quest, etc.
}
```

## Outcome

A `ScriptComponent` that attaches to any entity, four lifecycle hooks with deterministic execution order, persistent per-entity script state via Rhai scopes, and silent handling of missing hooks.

## Demo Integration

**Demo crate:** `nebula-demo`

Scripts are loaded from `assets/scripts/`. Each script has init, update, and shutdown phases that execute at appropriate times.

## Crates & Dependencies

| Crate | Version | Purpose |
|---|---|---|
| `rhai` | `1.23` | Script execution, AST inspection, Scope persistence |

## Unit Tests

```rust
#[test]
fn test_on_spawn_called_once_at_spawn() {
    let mut world = setup_test_world();
    let entity = spawn_entity_with_script(&mut world, "test_lifecycle.rhai");
    // First tick: on_spawn should execute
    run_script_system(&mut world);
    let script = world.get::<ScriptComponent>(entity).unwrap();
    let spawn_count: i64 = script.state.scope.get_value("spawn_count").unwrap();
    assert_eq!(spawn_count, 1);
    // Second tick: on_spawn should NOT execute again
    run_script_system(&mut world);
    let script = world.get::<ScriptComponent>(entity).unwrap();
    let spawn_count: i64 = script.state.scope.get_value("spawn_count").unwrap();
    assert_eq!(spawn_count, 1); // still 1, not 2
}

#[test]
fn test_on_update_called_each_frame() {
    let mut world = setup_test_world();
    let entity = spawn_entity_with_script(&mut world, "test_lifecycle.rhai");
    run_script_system(&mut world); // tick 1: on_spawn + on_update
    run_script_system(&mut world); // tick 2: on_update
    run_script_system(&mut world); // tick 3: on_update
    let script = world.get::<ScriptComponent>(entity).unwrap();
    let update_count: i64 = script.state.scope.get_value("update_count").unwrap();
    assert_eq!(update_count, 3);
}

#[test]
fn test_on_despawn_called_at_removal() {
    let mut world = setup_test_world();
    let entity = spawn_entity_with_script(&mut world, "test_lifecycle.rhai");
    run_script_system(&mut world); // initialize
    mark_for_despawn(&mut world, entity);
    run_script_system(&mut world); // should call on_despawn
    let script = world.get::<ScriptComponent>(entity).unwrap();
    let despawn_called: bool = script.state.scope.get_value("despawn_called").unwrap();
    assert!(despawn_called);
}

#[test]
fn test_state_persists_between_updates() {
    let mut world = setup_test_world();
    // Script: on_spawn sets counter=0; on_update increments counter
    let entity = spawn_entity_with_script(&mut world, "test_counter.rhai");
    run_script_system(&mut world); // spawn + update: counter = 1
    run_script_system(&mut world); // update: counter = 2
    let script = world.get::<ScriptComponent>(entity).unwrap();
    let counter: i64 = script.state.scope.get_value("counter").unwrap();
    assert_eq!(counter, 2);
}

#[test]
fn test_missing_hook_is_silently_skipped() {
    let mut world = setup_test_world();
    // Script only defines on_update, no on_spawn or on_despawn
    let entity = spawn_entity_with_script(&mut world, "test_update_only.rhai");
    // This should not panic or error
    run_script_system(&mut world);
    let script = world.get::<ScriptComponent>(entity).unwrap();
    assert_eq!(script.phase, ScriptPhase::Active);
}
```
