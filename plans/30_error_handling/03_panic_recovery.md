# Panic Recovery

## Problem

Rust panics are not exceptions -- they are signals that something has gone fundamentally wrong. But in a game engine, "fundamentally wrong" in one subsystem does not necessarily mean the entire process must abort. If the particle system panics due to an edge case in emitter logic, crashing the entire engine (destroying the player's unsaved progress, dropping their network connection, losing their work in the editor) is a disproportionate response. The particle system should be marked as failed and disabled, the panic should be logged with a full backtrace for debugging, and the game loop should continue without particles.

However, not all panics are recoverable. If the render loop panics, there is no meaningful way to continue -- the screen will not update. If the main thread panics in the ECS scheduler, the simulation is in an undefined state. These critical panics must still terminate the process, but they should do so with a helpful error message and backtrace rather than the default cryptic panic output.

Without panic recovery:

- **A single bug in an optional system kills the entire engine**, making development frustrating and gameplay fragile.
- **Panic messages are lost** -- The default panic hook writes to stderr, which may be invisible to users on Windows (no console window) or when running from a game launcher.
- **No post-mortem information** -- Without capturing the backtrace and logging it through the tracing infrastructure, developers have no way to diagnose panics that happen on user machines.

## Solution

### Custom Panic Hook

Install a custom panic hook at engine startup that logs panics through the `tracing` infrastructure instead of (or in addition to) writing to stderr. This ensures panics appear in the structured log file alongside all other engine events:

```rust
use std::panic;
use tracing;

pub fn install_panic_hook() {
    let default_hook = panic::take_hook();

    panic::set_hook(Box::new(move |panic_info| {
        // Extract the panic message
        let message = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "Unknown panic payload".to_string()
        };

        // Extract location (file, line, column)
        let location = panic_info.location().map(|loc| {
            format!("{}:{}:{}", loc.file(), loc.line(), loc.column())
        }).unwrap_or_else(|| "unknown location".to_string());

        // Capture backtrace
        let backtrace = std::backtrace::Backtrace::force_capture();

        // Log through tracing so it appears in the structured log file
        tracing::error!(
            panic.message = %message,
            panic.location = %location,
            panic.backtrace = %backtrace,
            "PANIC captured"
        );

        // Also invoke the default hook for stderr output
        default_hook(panic_info);
    }));
}
```

This hook is installed once in `main()` before any other initialization. It ensures every panic -- whether caught by `catch_unwind` or not -- is logged with full context.

### Catching Panics in Non-Critical Systems

Wrap non-critical ECS system execution in `std::panic::catch_unwind`. Because `catch_unwind` requires `UnwindSafe`, system functions must not hold references across the catch boundary. The wrapper takes ownership of the system function and catches any panic:

```rust
use std::panic::{catch_unwind, AssertUnwindSafe};

pub enum SystemResult {
    Ok,
    Panicked { message: String },
}

pub fn run_system_guarded<F>(
    system_name: &str,
    subsystem: Subsystem,
    f: F,
    degradation: &mut DegradationManager,
) -> SystemResult
where
    F: FnOnce() + std::panic::UnwindSafe,
{
    match catch_unwind(f) {
        Ok(()) => SystemResult::Ok,
        Err(payload) => {
            let message = if let Some(s) = payload.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "Unknown panic".to_string()
            };

            tracing::error!(
                system = system_name,
                subsystem = ?subsystem,
                panic_message = %message,
                "System panicked and has been disabled"
            );

            // Mark the subsystem as degraded in the DegradationManager
            // (see 02_graceful_degradation.md)
            let panic_error = PanicError { message: message.clone() };
            degradation.report_failure(subsystem, &panic_error);

            SystemResult::Panicked { message }
        }
    }
}
```

The `PanicError` is a simple wrapper to satisfy the `std::error::Error` trait requirement of `DegradationManager::report_failure`:

```rust
#[derive(Debug, thiserror::Error)]
#[error("panic in system: {message}")]
pub struct PanicError {
    pub message: String,
}
```

### Which Systems Get Guarded

Not every system is wrapped in `catch_unwind`. The overhead is minimal (it sets up an unwind landing pad), but the decision is based on criticality:

| System Category | Guarded? | Rationale |
|----------------|----------|-----------|
| Particle updates | Yes | Entirely cosmetic, safe to disable |
| Audio processing | Yes | Game is playable without sound |
| Debug overlays | Yes | Development-only, never critical |
| Script execution | Yes | User scripts should not crash the engine |
| Editor tools | Yes | Editor failures should not kill the runtime |
| Network send/recv | Yes | Can degrade to offline mode |
| Physics stepping | Conditional | Guarded in editor, unguarded in game |
| Chunk meshing | Conditional | Guarded if async, unguarded if synchronous |
| Render submission | No | Cannot continue without rendering |
| ECS schedule tick | No | Core loop, undefined state if it panics |
| Input processing | No | Cannot continue without input |

### System Disabling After Panic

Once a system panics, it is disabled for the remainder of the session. The game loop skips disabled systems on subsequent ticks:

```rust
pub struct SystemRegistry {
    systems: Vec<RegisteredSystem>,
}

struct RegisteredSystem {
    name: String,
    subsystem: Subsystem,
    enabled: bool,
    run_fn: Box<dyn FnMut() + Send>,
}

impl SystemRegistry {
    pub fn run_all(&mut self, degradation: &mut DegradationManager) {
        for system in &mut self.systems {
            if !system.enabled {
                continue; // Skip disabled systems
            }

            // Only guard non-critical systems
            if degradation.criticality(system.subsystem) != Criticality::Critical {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    (system.run_fn)();
                }));

                if let Err(payload) = result {
                    system.enabled = false;
                    let msg = extract_panic_message(&payload);
                    tracing::error!(
                        system = %system.name,
                        "System disabled after panic: {msg}"
                    );
                    let err = PanicError { message: msg };
                    degradation.report_failure(system.subsystem, &err);
                }
            } else {
                // Critical systems run unguarded -- if they panic, the process dies
                (system.run_fn)();
            }
        }
    }
}
```

### Critical Panic Behavior

When a critical system panics (or a panic occurs outside of a `catch_unwind` guard), the custom panic hook ensures:

1. The full backtrace is logged to the tracing infrastructure (and therefore to the log file).
2. The panic message is formatted clearly with the source location.
3. On desktop platforms, an error dialog can be shown (using `native-dialog` or `rfd`) before the process exits.
4. The log file is flushed to disk so post-mortem analysis is possible.

```rust
// In the panic hook, for critical (uncaught) panics:
tracing::error!(
    panic.message = %message,
    panic.location = %location,
    panic.backtrace = %backtrace,
    "CRITICAL PANIC - Engine shutting down"
);

// Flush the tracing subscriber to ensure logs are written
// (This depends on the subscriber implementation; for file-based
// subscribers, the file is flushed on drop.)
```

### UnwindSafe Considerations

`std::panic::catch_unwind` requires the closure to be `UnwindSafe`. In practice, most game systems hold mutable references to ECS resources, which are not `UnwindSafe`. The solution is to use `AssertUnwindSafe` at the call site, with the understanding that:

1. The panicking system is immediately disabled and will not run again.
2. Any partially-mutated state in that system's resources is accepted as potentially inconsistent.
3. The `DegradationManager` ensures no other system depends on a disabled system's output without checking the degradation status.

This is a deliberate trade-off: accepting potential inconsistency in a non-critical subsystem is better than crashing the entire engine.

## Outcome

Non-critical systems (particles, audio, scripting, debug overlays, editor tools, networking) are wrapped in `catch_unwind`. If they panic, the panic is captured, logged with a full backtrace through the tracing infrastructure, the system is disabled, and the game loop continues. Critical systems (render submission, ECS scheduling, input) run unguarded; if they panic, the custom panic hook logs the backtrace and the engine shuts down with a clear error message. All panic information is preserved in the structured log file for post-mortem debugging.

## Demo Integration

**Demo crate:** `nebula-demo`

A panic in a non-critical system (e.g., particles) is caught. The demo logs the panic, disables that system, and continues running with a brief notification message.

## Crates & Dependencies

- **`tracing = "0.1"`** -- Logging panic events with structured fields (message, location, backtrace) through the engine's existing logging infrastructure.
- **`thiserror = "2"`** -- Deriving `Error` on the `PanicError` wrapper type so it can be passed to `DegradationManager::report_failure()`.

No additional external crates are required. `std::panic::catch_unwind`, `std::panic::set_hook`, and `std::backtrace::Backtrace` are all in the standard library (Rust edition 2024, `Backtrace` is stable).

## Unit Tests

- **`test_non_critical_panic_is_caught`** -- Call `run_system_guarded` with a closure that panics (`panic!("test panic")`), subsystem `Subsystem::Particles`, and a `DegradationManager`. Assert the return value is `SystemResult::Panicked`. Assert the test does not abort. Assert the `DegradationManager` shows `Subsystem::Particles` as degraded.

- **`test_panic_message_is_captured`** -- Call `run_system_guarded` with `panic!("specific error message XYZ")`. Extract the message from the `SystemResult::Panicked` variant. Assert it contains "specific error message XYZ".

- **`test_panic_is_logged_with_backtrace`** -- Set up a tracing subscriber that writes to a buffer. Install the custom panic hook. Trigger a panic inside `catch_unwind`. Read the buffer contents and assert they contain "PANIC captured" or "System panicked". Assert the log output contains a backtrace (look for frame markers or file paths in the output).

- **`test_failed_system_is_disabled`** -- Create a `SystemRegistry` with a system that panics on first call. Run `run_all()` once -- the system panics and is disabled. Run `run_all()` again -- the system is skipped (it does not panic again, and the game loop completes without error). Assert the system's `enabled` field is `false` after the first run.

- **`test_game_loop_continues_after_panic`** -- Create a `SystemRegistry` with three systems: SystemA (increments a counter), SystemB (panics), SystemC (increments a counter). Run `run_all()`. Assert SystemA's counter is 1. Assert SystemB is disabled. Assert SystemC's counter is 1 (it was still executed despite SystemB panicking). Run `run_all()` again. Assert SystemA's counter is 2. Assert SystemC's counter is 2. Assert SystemB was skipped.

- **`test_critical_panic_is_not_caught`** -- Verify that `run_system_guarded` is not used for critical subsystems. Create a test where a critical system is run without `catch_unwind`. Use `std::panic::catch_unwind` at the test level to verify that the panic propagates (i.e., the system does not swallow it). This confirms that render and ECS panics will terminate the process.

- **`test_panic_error_implements_error_trait`** -- Construct a `PanicError { message: "test".into() }`. Assert it implements `std::error::Error` (call `.source()`, `.to_string()`). Assert the display output is `"panic in system: test"`.

- **`test_assert_unwind_safe_is_required`** -- Verify at compile time (this is a compile-test) that `catch_unwind` with a closure capturing `&mut Vec<i32>` does not compile without `AssertUnwindSafe`. This documents the intentional use of `AssertUnwindSafe` in the codebase.
