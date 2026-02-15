# Event Hooks

## Problem

The lifecycle hooks (`on_spawn`, `on_update`, `on_despawn`) cover entity-scoped behavior, but gameplay often requires reacting to world events: a voxel changing nearby, a player entering an area, damage being received, or a timed delay expiring. Without an event system, scripts would have to poll for state changes every frame, which is wasteful and error-prone. Scripts need a way to register callbacks that fire when specific events occur.

## Solution

### Event Registry

A centralized `ScriptEventRegistry` stores callback registrations:

```rust
#[derive(Resource)]
pub struct ScriptEventRegistry {
    /// Map from event name to list of registered listeners
    pub listeners: HashMap<String, Vec<ScriptEventListener>>,
    /// Active timers
    pub timers: Vec<ScriptTimer>,
    /// Monotonically increasing callback ID for unregistration
    next_callback_id: u64,
}

pub struct ScriptEventListener {
    pub callback_id: u64,
    /// Entity the script is attached to (None for global scripts)
    pub entity: Option<EntityId>,
    /// Name of the Rhai function to call
    pub callback_fn: String,
    /// Reference to the AST containing the callback
    pub ast: Arc<AST>,
    /// Scope for persistent state
    pub scope: Arc<Mutex<rhai::Scope<'static>>>,
}

pub struct ScriptTimer {
    pub callback_id: u64,
    pub entity: Option<EntityId>,
    pub remaining: f64,
    pub callback_fn: String,
    pub ast: Arc<AST>,
    pub scope: Arc<Mutex<rhai::Scope<'static>>>,
    pub repeating: bool,
    pub interval: f64,
}
```

### Registration Functions

These functions are called from within Rhai scripts:

```rhai
// Register for voxel change events within a radius
on_voxel_changed(fn(pos, old_type, new_type) {
    // react to terrain modification
});

// Register for player proximity events
on_player_entered_area(fn(player_id) {
    // greet, trigger quest, open door
});

// Register for damage events on the current entity
on_damage_taken(fn(amount, source) {
    // flash red, play sound, check death
});

// Fire a callback after a delay (seconds)
on_timer(2.5, fn() {
    // delayed effect
});

// Repeating timer
on_timer_repeat(1.0, fn() {
    // periodic pulse
});
```

Under the hood each registration function pushes a `ScriptEventListener` or `ScriptTimer` into the `ScriptEventRegistry`.

### Event Dispatch System

The `dispatch_script_events` system runs after the main game systems and before rendering:

```rust
fn dispatch_script_events(
    mut registry: ResMut<ScriptEventRegistry>,
    events: Res<GameEvents>,
    engine: Res<ScriptEngine>,
    time: Res<Time>,
) {
    // 1. Dispatch voxel change events
    for event in events.voxel_changes.iter() {
        if let Some(listeners) = registry.listeners.get("voxel_changed") {
            for listener in listeners {
                // Check if event is relevant to this listener (distance, etc.)
                engine.call_fn_with_timeout(
                    &mut listener.scope.lock(),
                    &listener.ast,
                    &listener.callback_fn,
                    (event.position, event.old_type, event.new_type),
                );
            }
        }
    }

    // 2. Dispatch player proximity events
    for event in events.player_area_entries.iter() {
        // similar dispatch pattern
    }

    // 3. Dispatch damage events
    for event in events.damage_events.iter() {
        // similar dispatch pattern
    }

    // 4. Tick timers
    let dt = time.delta_seconds_f64();
    registry.timers.retain_mut(|timer| {
        timer.remaining -= dt;
        if timer.remaining <= 0.0 {
            engine.call_fn_with_timeout(
                &mut timer.scope.lock(),
                &timer.ast,
                &timer.callback_fn,
                (),
            );
            if timer.repeating {
                timer.remaining = timer.interval;
                true // keep the timer
            } else {
                false // remove one-shot timer
            }
        } else {
            true // keep, not yet elapsed
        }
    });
}
```

### Global Scripts

Scripts not attached to any entity can be loaded as global scripts. These are useful for world-level logic like day/night cycles, weather, or server rules. Global scripts register with `entity: None` and their callbacks persist for the lifetime of the script:

```rust
#[derive(Resource)]
pub struct GlobalScripts {
    pub scripts: Vec<GlobalScriptInstance>,
}

pub struct GlobalScriptInstance {
    pub source: String,
    pub ast: Arc<AST>,
    pub scope: rhai::Scope<'static>,
}
```

### Unregistration

Callbacks can be unregistered by storing the returned callback ID:

```rhai
let id = on_timer(5.0, fn() { /* ... */ });
cancel_callback(id);
```

When an entity with registered callbacks is despawned, all its callbacks are automatically cleaned up by the despawn system.

## Outcome

An event hook system that allows scripts to register callbacks for voxel changes, player proximity, damage, and timed delays. A dispatch system that fires callbacks with event data each frame. Support for global scripts and automatic cleanup on entity despawn.

## Demo Integration

**Demo crate:** `nebula-demo`

Scripts subscribe to game events. An `on_voxel_break` hook triggers a script that spawns particle effects at the break location.

## Crates & Dependencies

| Crate | Version | Purpose |
|---|---|---|
| `rhai` | `1.23` | Callback function execution |
| `parking_lot` | `0.12` | Mutex for shared scope access during dispatch |

## Unit Tests

```rust
#[test]
fn test_event_fires_callback() {
    let mut world = setup_test_world();
    let entity = spawn_entity_with_script(&mut world, "test_events.rhai");
    // Script registers: on_voxel_changed(fn(pos, old, new) { changed = true; })
    run_script_system(&mut world); // initializes and registers callback

    // Emit a voxel change event
    emit_voxel_change_event(&mut world, ScriptVec3::new(1.0, 0.0, 0.0), VoxelTypeId::AIR, VoxelTypeId::STONE);
    dispatch_script_events(&mut world);

    let registry = world.resource::<ScriptEventRegistry>();
    let listener = &registry.listeners["voxel_changed"][0];
    let changed: bool = listener.scope.lock().get_value("changed").unwrap();
    assert!(changed);
}

#[test]
fn test_timer_fires_after_delay() {
    let mut world = setup_test_world();
    let entity = spawn_entity_with_script(&mut world, "test_timer.rhai");
    // Script registers: on_timer(1.0, fn() { timer_fired = true; })
    run_script_system(&mut world);

    // Advance time by 0.5s -- timer should NOT have fired
    advance_time(&mut world, 0.5);
    dispatch_script_events(&mut world);
    let fired = get_script_var::<bool>(&world, entity, "timer_fired");
    assert!(!fired);

    // Advance time by another 0.6s (total 1.1s) -- timer should fire
    advance_time(&mut world, 0.6);
    dispatch_script_events(&mut world);
    let fired = get_script_var::<bool>(&world, entity, "timer_fired");
    assert!(fired);
}

#[test]
fn test_multiple_callbacks_on_same_event() {
    let mut world = setup_test_world();
    let e1 = spawn_entity_with_script(&mut world, "test_event_a.rhai");
    let e2 = spawn_entity_with_script(&mut world, "test_event_b.rhai");
    run_script_system(&mut world); // both register on_voxel_changed

    emit_voxel_change_event(&mut world, ScriptVec3::ZERO, VoxelTypeId::AIR, VoxelTypeId::STONE);
    dispatch_script_events(&mut world);

    let a_fired = get_script_var::<bool>(&world, e1, "callback_fired");
    let b_fired = get_script_var::<bool>(&world, e2, "callback_fired");
    assert!(a_fired);
    assert!(b_fired);
}

#[test]
fn test_unregistered_callback_does_not_fire() {
    let mut world = setup_test_world();
    let entity = spawn_entity_with_script(&mut world, "test_cancel.rhai");
    // Script registers a callback and then immediately cancels it
    run_script_system(&mut world);

    emit_voxel_change_event(&mut world, ScriptVec3::ZERO, VoxelTypeId::AIR, VoxelTypeId::STONE);
    dispatch_script_events(&mut world);

    let fired = get_script_var::<bool>(&world, entity, "callback_fired");
    assert!(!fired); // callback was cancelled, should not fire
}

#[test]
fn test_event_data_is_correct() {
    let mut world = setup_test_world();
    let entity = spawn_entity_with_script(&mut world, "test_event_data.rhai");
    // Script: on_damage_taken(fn(amount, source) { last_damage = amount; })
    run_script_system(&mut world);

    emit_damage_event(&mut world, entity, 25.0, ScriptEntityId(999));
    dispatch_script_events(&mut world);

    let last_damage = get_script_var::<f64>(&world, entity, "last_damage");
    assert!((last_damage - 25.0).abs() < f64::EPSILON);
}
```
