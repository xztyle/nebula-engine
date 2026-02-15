# Script Hot Reloading

## Problem

During development, requiring an engine restart to test script changes kills iteration speed. Game designers and modders need to edit `.rhai` files in an external editor, save, and see the changes take effect immediately in the running engine. However, hot reloading scripts is non-trivial: the old AST must be replaced, entities referencing the script must pick up the new version, and ideally script state should survive the reload so the developer does not have to manually recreate the game situation they were testing.

## Solution

### Filesystem Watcher

Use the `notify` crate (the same filesystem watcher used for asset hot-reload elsewhere in the engine) to watch the `scripts/` directory for file modifications:

```rust
#[derive(Resource)]
pub struct ScriptFileWatcher {
    pub watcher: RecommendedWatcher,
    pub change_rx: Receiver<ScriptFileEvent>,
    pub enabled: bool,
}

pub enum ScriptFileEvent {
    Modified(PathBuf),
    Created(PathBuf),
    Removed(PathBuf),
}
```

The watcher is initialized during `ScriptPlugin::build` and is only active when the engine is running in development mode (`#[cfg(debug_assertions)]` or a runtime flag). In release builds the watcher is never created, imposing zero overhead.

### Reload Pipeline

When a `.rhai` file modification is detected:

1. **Debounce**: File system events are debounced with a 100ms window to avoid reacting to partial writes from text editors that save in multiple steps (write temp, rename).

2. **Recompile**: The modified file is recompiled via `engine.compile_file()`. If compilation fails, a warning is logged with the full error message (including line/column) and the old AST is kept. The game continues running with the previous working version.

3. **Replace AST**: On successful compilation, the new `Arc<AST>` replaces the old one in `ScriptEngine::scripts`.

4. **Update Entities**: All entities whose `ScriptComponent::source` matches the changed file path receive the new AST on their next execution. Since the system looks up the AST by path from `ScriptEngine::scripts` each frame, the update is automatic -- no explicit entity iteration is needed.

5. **State Preservation**: The entity's `ScriptState::scope` is preserved across the reload. After replacing the AST, the system calls `on_spawn` again if the function exists in the new AST, passing a special `is_reload` flag so the script can decide whether to reinitialize or keep existing state:

```rhai
fn on_spawn() {
    if is_reload {
        // Keep existing state, just re-bind references
        print("Script reloaded, state preserved");
    } else {
        // Fresh initialization
        let health = 100.0;
        let counter = 0;
    }
}
```

### Reload Notification

A `ScriptReloaded` event is emitted when a script is successfully reloaded, containing the file path and a list of affected entity IDs. The debug overlay can subscribe to this event to show a brief notification ("Reloaded door_logic.rhai - 3 entities updated").

### Error Reporting

Compilation errors during hot reload are surfaced through multiple channels:

- **Log**: `warn!("Script compilation failed: scripts/door.rhai:12:5 - variable 'x' not found")`
- **Debug overlay**: If the in-game debug UI is active, a persistent error banner appears until the script compiles successfully.
- **File**: Errors are also written to `logs/script_errors.log` for reference.

### Disabling Hot Reload

Hot reload can be disabled at runtime via a console command or configuration flag:

```rust
// In config.toml
[scripting]
hot_reload = true   # default in dev, false in release

// At runtime
scripting.hot_reload = false
```

When disabled, the filesystem watcher is dropped and no polling occurs.

## Outcome

A development-time hot reload system that watches `.rhai` files for changes, recompiles on save, replaces the AST across all referencing entities, preserves script state where possible, and reports compilation errors without crashing the engine.

## Demo Integration

**Demo crate:** `nebula-demo`

Editing a `.rhai` script file on disk causes the running demo to detect the change and reload the script within 1 second.

## Crates & Dependencies

| Crate | Version | Purpose |
|---|---|---|
| `rhai` | `1.23` | AST compilation and replacement |
| `notify` | `7` | Cross-platform filesystem watcher |
| `crossbeam-channel` | `0.5` | Lock-free channel for watcher events |
| `log` | `0.4` | Warning/error logging |

## Unit Tests

```rust
#[test]
fn test_file_change_triggers_reload() {
    let mut world = setup_test_world_with_watcher();
    let entity = spawn_entity_with_script(&mut world, "test_reload.rhai");
    run_script_system(&mut world);

    // Simulate file modification event
    let script_path = test_scripts_dir().join("test_reload.rhai");
    simulate_file_change(&mut world, &script_path);
    process_reload_events(&mut world);

    let engine = world.resource::<ScriptEngine>();
    // The AST should have been recompiled (version counter incremented)
    assert!(engine.was_recently_reloaded("test_reload.rhai"));
}

#[test]
fn test_recompiled_script_takes_effect() {
    let mut world = setup_test_world_with_watcher();
    let entity = spawn_entity_with_script(&mut world, "test_value.rhai");
    // Original script: on_update sets result = 1
    run_script_system(&mut world);
    let val = get_script_var::<i64>(&world, entity, "result");
    assert_eq!(val, 1);

    // Rewrite script: on_update sets result = 42
    write_script_file("test_value.rhai", "fn on_update(dt) { result = 42; }");
    simulate_file_change(&mut world, &test_scripts_dir().join("test_value.rhai"));
    process_reload_events(&mut world);
    run_script_system(&mut world);

    let val = get_script_var::<i64>(&world, entity, "result");
    assert_eq!(val, 42);
}

#[test]
fn test_syntax_error_reported_without_crash() {
    let mut world = setup_test_world_with_watcher();
    let entity = spawn_entity_with_script(&mut world, "test_error.rhai");
    run_script_system(&mut world);

    // Introduce a syntax error
    write_script_file("test_error.rhai", "fn on_update(dt { }"); // missing closing paren
    simulate_file_change(&mut world, &test_scripts_dir().join("test_error.rhai"));
    process_reload_events(&mut world); // should not panic

    // The old working AST should still be in place
    run_script_system(&mut world); // should continue running without crash
    let engine = world.resource::<ScriptEngine>();
    assert!(engine.has_reload_error("test_error.rhai"));
}

#[test]
fn test_state_preserved_if_possible() {
    let mut world = setup_test_world_with_watcher();
    let entity = spawn_entity_with_script(&mut world, "test_preserve.rhai");
    // Script: on_spawn sets counter=0; on_update increments counter
    run_script_system(&mut world); // counter = 0 + 1 = 1
    run_script_system(&mut world); // counter = 2
    run_script_system(&mut world); // counter = 3

    // Hot-reload the script (same logic, minor change elsewhere)
    write_script_file("test_preserve.rhai", UPDATED_PRESERVE_SCRIPT);
    simulate_file_change(&mut world, &test_scripts_dir().join("test_preserve.rhai"));
    process_reload_events(&mut world);
    run_script_system(&mut world); // counter should be 4, not reset to 1

    let counter = get_script_var::<i64>(&world, entity, "counter");
    assert_eq!(counter, 4); // state preserved across reload
}

#[test]
fn test_hot_reload_can_be_disabled() {
    let mut world = setup_test_world_with_watcher();
    disable_hot_reload(&mut world);

    write_script_file("test_disabled.rhai", "fn on_update(dt) { result = 99; }");
    simulate_file_change(&mut world, &test_scripts_dir().join("test_disabled.rhai"));
    process_reload_events(&mut world);

    // Watcher should be inactive, no reload should have occurred
    let engine = world.resource::<ScriptEngine>();
    assert!(!engine.was_recently_reloaded("test_disabled.rhai"));
}
```
