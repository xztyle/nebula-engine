# Script Sandboxing

## Problem

Nebula Engine supports modding through Rhai scripts, which means untrusted code will run inside the engine process. A malicious or buggy script must not be able to: access the filesystem, open network connections, crash the engine via stack overflow, exhaust memory, or call internal engine functions that were not explicitly exposed. The scripting sandbox must be airtight by default, with any capability granted only through deliberate API registration.

## Solution

### Rhai's Built-in Sandbox

Rhai is designed as an embedded scripting language and does **not** provide filesystem, networking, or OS access by default. There are no built-in functions for `open`, `read`, `write`, `exec`, `socket`, or any other system call. This is the first and strongest layer of defense -- scripts simply cannot express dangerous operations unless the host explicitly registers them.

To reinforce this, the engine does **not** register any of Rhai's optional packages that could introduce unsafe behavior:

```rust
let mut engine = Engine::new_raw(); // no default packages
engine.register_global_module(rhai::packages::StandardPackage::new().as_shared_module());
// Deliberately NOT registering:
// - FilesystemPackage (does not exist in rhai, but listed for clarity)
// - Any custom IO functions
```

### Execution Limits

Multiple layers of execution limits prevent resource exhaustion:

| Limit | Setting | Effect |
|---|---|---|
| **Max operations** | `engine.set_max_operations(100_000)` | Caps total bytecode operations per invocation |
| **Max call depth** | `engine.set_max_call_levels(64)` | Prevents stack overflow from deep/infinite recursion |
| **Max expression depth** | `engine.set_max_expr_depths(64, 32)` | Limits AST nesting (prevents pathological parsing) |
| **Max string size** | `engine.set_max_string_size(16_384)` | Prevents memory bombs via string concatenation |
| **Max array size** | `engine.set_max_array_size(4_096)` | Prevents memory bombs via array growth |
| **Max map size** | `engine.set_max_map_size(1_024)` | Prevents memory bombs via object-map growth |
| **Wall-clock timeout** | `tokio::time::timeout(10ms, ...)` | Hard kill for scripts that somehow evade operation limits |

### Progress Callback

The `on_progress` callback provides fine-grained control and is the mechanism that enforces the operation limit:

```rust
engine.on_progress(|ops| {
    if ops > 100_000 {
        Some(EvalAltResult::ErrorTerminated(
            Dynamic::from("operation limit exceeded"),
            Position::NONE,
        ).into())
    } else {
        None
    }
});
```

### Memory Limiting

Rhai does not have a built-in memory allocator hook, so memory limiting is achieved through the combination of max string/array/map sizes above and a per-script memory estimation system:

```rust
pub struct ScriptMemoryTracker {
    /// Estimated memory usage per script instance
    pub usage: HashMap<EntityId, usize>,
    /// Maximum allowed per script (default: 1 MiB)
    pub max_per_script: usize,
}
```

The tracker is updated after each script call by inspecting the scope's variable count and estimating sizes. If a script exceeds its budget, it is terminated and its entity receives a `ScriptError` component.

### Restricted Scope

Scripts execute in a scope that contains **only** the variables and functions explicitly provided:

```rust
fn build_restricted_scope(ctx: &ScriptContext) -> rhai::Scope<'static> {
    let mut scope = rhai::Scope::new();
    // Only the API functions registered on the Engine are available.
    // No ambient variables, no global state leakage.
    scope
}
```

The engine's function namespace contains only:
- ECS query functions (`get_position`, `set_position`, `get_entities_near`, etc.)
- Voxel functions (`get_voxel`, `set_voxel`, `raycast`, etc.)
- Event functions (`on_voxel_changed`, `on_timer`, etc.)
- Ability functions (`define_ability`, `apply_damage`, etc.)
- Math utilities (`Vec3` constructors, `distance`, `normalize`, etc.)
- Print/debug (`print`, `debug` -- routed to engine log, not stdout)

No other functions exist in the Rhai engine's namespace. A script cannot call anything that is not in this list.

### Print/Debug Redirection

The `print` and `debug` statements are redirected to the engine's logging system:

```rust
engine.on_print(|text| {
    log::info!(target: "rhai_script", "{}", text);
});
engine.on_debug(|text, source, pos| {
    log::debug!(target: "rhai_script", "[{:?}:{}] {}", source, pos, text);
});
```

This prevents scripts from writing to stdout/stderr, which could interfere with the engine's terminal output or be used as a side channel.

### Script Permission Levels

Different scripts can be granted different permission levels:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptPermission {
    /// Can only read world state, no mutations
    ReadOnly,
    /// Can mutate entities and voxels within range
    LocalWrite,
    /// Full access (admin/creative mode, server scripts)
    WorldWrite,
}
```

The permission level is checked inside each mutation API function before accepting the command into the buffer.

### Error Recovery

When a script is terminated (timeout, operation limit, stack overflow, memory limit), the engine:

1. Logs the error with script path, entity ID, and error details.
2. Adds a `ScriptError` component to the entity for inspection.
3. Does **not** crash or panic -- the frame continues with the remaining scripts.
4. Optionally disables the script after N consecutive errors (configurable, default: 5).

```rust
#[derive(Component)]
pub struct ScriptError {
    pub message: String,
    pub timestamp: f64,
    pub consecutive_failures: u32,
}
```

## Outcome

A multi-layered sandboxing system that prevents scripts from accessing the filesystem or network (Rhai's design), limits CPU usage (operation count + wall-clock timeout), limits memory usage (string/array/map caps + memory tracking), restricts the API surface to explicitly registered functions, and recovers gracefully from script errors without engine crashes.

## Demo Integration

**Demo crate:** `nebula-demo`

Malicious script operations like file I/O are blocked by the sandbox. Console logs `Sandbox violation: file I/O denied`.

## Crates & Dependencies

| Crate | Version | Purpose |
|---|---|---|
| `rhai` | `1.23` | Sandboxed scripting engine with configurable limits |
| `tokio` | `1` | Wall-clock timeout enforcement |
| `log` | `0.4` | Error and debug logging |

## Unit Tests

```rust
#[test]
fn test_filesystem_access_is_blocked() {
    let se = ScriptEngine::new("tests/scripts");
    // Rhai has no filesystem functions, so any attempt to access files
    // should result in a function-not-found error
    let result = se.engine.eval::<Dynamic>(r#"open("etc/passwd")"#);
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("not found") || err.contains("Unknown"));
}

#[test]
fn test_infinite_loop_killed_by_timeout() {
    let se = ScriptEngine::new("tests/scripts");
    let ast = se.engine.compile("loop { let x = 1; }").unwrap();
    let result = se.eval_with_timeout(&ast, Duration::from_millis(10));
    assert!(result.is_err());
    // Should terminate, not hang
}

#[test]
fn test_stack_overflow_caught() {
    let se = ScriptEngine::new("tests/scripts");
    let ast = se.engine.compile(r#"
        fn recurse(n) {
            recurse(n + 1)
        }
        recurse(0)
    "#).unwrap();
    let result: Result<Dynamic, _> = se.engine.eval_ast(&ast);
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("call") || err.contains("stack") || err.contains("limit"));
}

#[test]
fn test_memory_limit_prevents_large_allocations() {
    let se = ScriptEngine::new("tests/scripts");
    // Attempt to create a string larger than the 16 KiB limit
    let ast = se.engine.compile(r#"
        let s = "x";
        for i in 0..20 {
            s += s;  // doubles each iteration: 2^20 = 1 MiB
        }
    "#).unwrap();
    let result: Result<Dynamic, _> = se.engine.eval_ast(&ast);
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("size") || err.contains("limit") || err.contains("too large"));
}

#[test]
fn test_only_registered_apis_are_accessible() {
    let se = ScriptEngine::new("tests/scripts");
    // Verify that a registered function works
    let ok_result = se.engine.eval::<f64>("let v = Vec3(1.0, 2.0, 3.0); v.x");
    assert!(ok_result.is_ok());

    // Verify that an unregistered function fails
    let bad_result = se.engine.eval::<Dynamic>("exec(\"rm -rf /\")");
    assert!(bad_result.is_err());

    // Verify that internal engine functions are not accessible
    let internal_result = se.engine.eval::<Dynamic>("__internal_shutdown()");
    assert!(internal_result.is_err());
}
```
