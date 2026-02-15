# Rhai Engine Initialization

## Problem

Nebula Engine needs an embedded scripting layer so that gameplay logic, modding, and rapid iteration can happen without recompiling the Rust engine. The scripting engine must be safe, deterministic enough for a game loop, and tightly integrated with the engine's ECS and 128-bit coordinate system. Without resource limits a single rogue script could freeze the entire game, so the engine must enforce strict execution budgets from the very first initialization.

## Solution

Initialize a `rhai::Engine` (v1.23) as a singleton ECS resource (`Res<ScriptEngine>`) during the engine's startup phase.

### Configuration

| Limit | Value | Rationale |
|---|---|---|
| `set_max_call_levels` | 64 | Prevent unbounded recursion |
| `set_max_operations` | 100_000 | Cap CPU work per invocation |
| `set_max_string_size` | 16_384 (16 KiB) | Prevent memory bombs via string concat |
| `set_max_array_size` | 4_096 | Prevent memory bombs via array growth |
| `set_max_map_size` | 1_024 | Limit object-map allocations |
| `set_max_expr_depths` | (64, 32) | Global / function expression nesting |

A 10 ms wall-clock execution timeout is enforced via `tokio::time::timeout` wrapping each `Engine::call_fn` / `Engine::eval_ast` invocation. Rhai's own `on_progress` callback is also set to bail after `max_operations` so the engine can abort even inside tight loops that do not yield to async.

### Custom Type Registration

Three core types are registered so scripts can work with engine primitives natively:

```rust
// Vec3 -- wraps the engine's 128-bit coordinate vector
#[derive(Debug, Clone, CustomType)]
pub struct ScriptVec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

// VoxelTypeId -- opaque handle into the voxel registry
#[derive(Debug, Clone, CustomType)]
pub struct ScriptVoxelTypeId(pub u32);

// EntityId -- opaque ECS entity handle
#[derive(Debug, Clone, CustomType)]
pub struct ScriptEntityId(pub u64);
```

Each type is registered with `engine.build_type::<T>()`. Arithmetic operators (`+`, `-`, `*`) and comparison operators (`==`, `!=`) are registered for `ScriptVec3`. Display implementations are registered so `print(vec)` works in scripts.

### Script Loading

On startup a `ScriptAssetLoader` scans the `scripts/` directory (relative to the game root) for `*.rhai` files, compiles each to an `AST`, and stores them in a `HashMap<String, Arc<AST>>` keyed by the relative path. Compilation errors are logged as warnings but do not halt engine startup -- a sentinel `AST` that returns an error value is stored instead.

### ECS Integration

```rust
#[derive(Resource)]
pub struct ScriptEngine {
    pub engine: Engine,
    pub scripts: HashMap<String, Arc<AST>>,
}
```

The resource is inserted during the `ScriptPlugin::build` method, which runs in the `Startup` schedule.

## Outcome

A fully configured `ScriptEngine` ECS resource available to all systems, with compiled ASTs for every `.rhai` file in the `scripts/` directory, strict execution limits, and three registered custom types ready for interop.

## Demo Integration

**Demo crate:** `nebula-demo`

The Rhai scripting engine initializes at startup. Console logs `Rhai v1.23 initialized` confirming the scripting layer is active.

## Crates & Dependencies

| Crate | Version | Purpose |
|---|---|---|
| `rhai` | `1.23` | Embedded scripting engine |
| `tokio` | `1` | Async runtime for wall-clock timeout |
| `notify` | `7` | Filesystem watcher (used later for hot-reload) |
| `log` | `0.4` | Logging compilation warnings |

## Unit Tests

```rust
#[test]
fn test_engine_initializes_with_defaults() {
    let se = ScriptEngine::new("tests/scripts");
    // Engine should exist and limits should be set
    assert_eq!(se.engine.max_call_levels(), 64);
    assert_eq!(se.engine.max_operations(), Some(100_000));
    assert_eq!(se.engine.max_string_size(), Some(16_384));
    assert_eq!(se.engine.max_array_size(), Some(4_096));
}

#[test]
fn test_timeout_kills_long_script() {
    let se = ScriptEngine::new("tests/scripts");
    let ast = se.engine.compile("loop { }").unwrap();
    let result = se.eval_with_timeout(&ast, Duration::from_millis(10));
    assert!(result.is_err());
    // The error should indicate a timeout or operation-limit breach
    let err_str = format!("{:?}", result.unwrap_err());
    assert!(err_str.contains("progress") || err_str.contains("timeout"));
}

#[test]
fn test_custom_types_registered() {
    let se = ScriptEngine::new("tests/scripts");
    let ast = se.engine.compile(r#"
        let v = Vec3(1.0, 2.0, 3.0);
        v.x + v.y + v.z
    "#).unwrap();
    let result: f64 = se.engine.eval_ast(&ast).unwrap();
    assert!((result - 6.0).abs() < f64::EPSILON);
}

#[test]
fn test_script_compiles_from_directory() {
    // Place a valid .rhai file in tests/scripts/hello.rhai
    let se = ScriptEngine::new("tests/scripts");
    assert!(se.scripts.contains_key("hello.rhai"));
    assert!(se.scripts.get("hello.rhai").is_some());
}

#[test]
fn test_script_execution_returns_result() {
    let se = ScriptEngine::new("tests/scripts");
    let ast = se.engine.compile("40 + 2").unwrap();
    let result: i64 = se.engine.eval_ast(&ast).unwrap();
    assert_eq!(result, 42);
}
```
