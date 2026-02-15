# Graceful Degradation

## Problem

A game engine is a composite of many subsystems, and not all of them are equally critical. If the audio backend fails to initialize because the user has no audio device, the engine should not crash -- it should continue running silently. If a shader fails to compile, the engine should render something (a solid magenta fallback) rather than presenting a black screen or panicking. If the network connection drops during a multiplayer session, the player should transition to a degraded single-player mode rather than being kicked to the main menu with a cryptic error.

Without a degradation strategy, every subsystem failure is treated as fatal. This leads to a fragile engine where:

- **Users on unusual hardware lose access entirely** -- A machine with no dedicated GPU, a missing audio driver, or a firewall blocking game ports becomes unusable instead of merely limited.
- **Transient failures become permanent** -- A momentary network hiccup or a single corrupt asset file crashes the entire session, destroying unsaved progress.
- **Development is slowed** -- Developers working on the terrain system cannot test their changes if the audio subsystem is broken on their branch, because the engine crashes at startup.

The engine needs a `DegradationManager` that tracks which subsystems are healthy, which have failed, and what fallback behavior is active. Non-critical failures are logged and reported to the user via the UI, but the game loop continues.

## Solution

### Subsystem Criticality Classification

Classify every subsystem into one of three tiers:

| Tier | Behavior on Failure | Examples |
|------|---------------------|----------|
| **Critical** | Engine shuts down gracefully with an error message | Render device initialization, main window creation, ECS world setup |
| **Important** | Engine continues but notifies the user prominently | Physics engine, chunk loading, input system |
| **Optional** | Engine continues silently (or with a subtle indicator) | Audio, networking, particles, debug overlays, editor tools |

### DegradationManager

A central resource (ECS resource or globally accessible singleton) that tracks subsystem health:

```rust
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Subsystem {
    Render,
    Audio,
    Physics,
    Network,
    ChunkLoading,
    Input,
    Particles,
    Scripting,
    Editor,
    Debug,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubsystemStatus {
    /// Subsystem is operating normally.
    Healthy,
    /// Subsystem has failed but a fallback is active.
    Degraded { since_tick: u64 },
    /// Subsystem has been manually disabled.
    Disabled,
    /// Subsystem has recovered from a previous failure.
    Recovered { degraded_for_ticks: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Criticality {
    Critical,
    Important,
    Optional,
}

pub struct DegradationManager {
    statuses: HashMap<Subsystem, SubsystemStatus>,
    criticalities: HashMap<Subsystem, Criticality>,
    current_tick: u64,
}

impl DegradationManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            statuses: HashMap::new(),
            criticalities: HashMap::new(),
            current_tick: 0,
        };
        // Register all subsystems with their criticality and initial healthy status
        mgr.register(Subsystem::Render, Criticality::Critical);
        mgr.register(Subsystem::Audio, Criticality::Optional);
        mgr.register(Subsystem::Physics, Criticality::Important);
        mgr.register(Subsystem::Network, Criticality::Optional);
        mgr.register(Subsystem::ChunkLoading, Criticality::Important);
        mgr.register(Subsystem::Input, Criticality::Important);
        mgr.register(Subsystem::Particles, Criticality::Optional);
        mgr.register(Subsystem::Scripting, Criticality::Optional);
        mgr.register(Subsystem::Editor, Criticality::Optional);
        mgr.register(Subsystem::Debug, Criticality::Optional);
        mgr
    }

    fn register(&mut self, subsystem: Subsystem, criticality: Criticality) {
        self.statuses.insert(subsystem, SubsystemStatus::Healthy);
        self.criticalities.insert(subsystem, criticality);
    }

    /// Report a subsystem failure. Returns `true` if the engine should shut down
    /// (i.e., the failed subsystem is critical).
    pub fn report_failure(&mut self, subsystem: Subsystem, error: &dyn std::error::Error) -> bool {
        let criticality = self.criticalities.get(&subsystem).copied()
            .unwrap_or(Criticality::Optional);

        tracing::error!(
            subsystem = ?subsystem,
            criticality = ?criticality,
            error = %error,
            "Subsystem failure reported"
        );

        self.statuses.insert(subsystem, SubsystemStatus::Degraded {
            since_tick: self.current_tick,
        });

        criticality == Criticality::Critical
    }

    /// Report that a subsystem has recovered from a degraded state.
    pub fn report_recovery(&mut self, subsystem: Subsystem) {
        if let Some(SubsystemStatus::Degraded { since_tick }) = self.statuses.get(&subsystem) {
            let degraded_for = self.current_tick.saturating_sub(*since_tick);
            tracing::info!(
                subsystem = ?subsystem,
                degraded_for_ticks = degraded_for,
                "Subsystem recovered"
            );
            self.statuses.insert(subsystem, SubsystemStatus::Recovered {
                degraded_for_ticks: degraded_for,
            });
        }
    }

    pub fn is_healthy(&self, subsystem: Subsystem) -> bool {
        matches!(
            self.statuses.get(&subsystem),
            Some(SubsystemStatus::Healthy) | Some(SubsystemStatus::Recovered { .. })
        )
    }

    pub fn is_degraded(&self, subsystem: Subsystem) -> bool {
        matches!(self.statuses.get(&subsystem), Some(SubsystemStatus::Degraded { .. }))
    }

    pub fn status(&self, subsystem: Subsystem) -> SubsystemStatus {
        self.statuses.get(&subsystem).copied().unwrap_or(SubsystemStatus::Healthy)
    }

    pub fn all_degraded(&self) -> Vec<(Subsystem, SubsystemStatus)> {
        self.statuses.iter()
            .filter(|(_, s)| matches!(s, SubsystemStatus::Degraded { .. }))
            .map(|(k, v)| (*k, *v))
            .collect()
    }

    pub fn tick(&mut self) {
        self.current_tick += 1;
    }
}
```

### Concrete Degradation Scenarios

#### Shader Compilation Failure

When a shader fails to compile, the render system catches the error and substitutes a fallback solid-color shader. The fallback shader is compiled once at engine startup and is guaranteed to work (it uses only basic vertex transformation and a uniform color output):

```rust
pub fn compile_shader_or_fallback(
    device: &wgpu::Device,
    source: &str,
    name: &str,
    fallback: &wgpu::ShaderModule,
    degradation: &mut DegradationManager,
) -> wgpu::ShaderModule {
    match device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(name),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    }) {
        module => {
            // wgpu may report errors asynchronously via the error callback,
            // but the module is still returned. Check for compilation errors
            // through validation or a dedicated error scope.
            module
        }
    }
    // In practice, use device.push_error_scope / pop_error_scope to detect
    // compilation failures and fall back to the pre-compiled fallback module.
}
```

The fallback shader renders affected geometry in a bright magenta color (`#FF00FF`), the universally recognized "something is wrong" signal in game development.

#### Audio Device Missing

During audio initialization, if no audio device is available, the audio system enters a no-op mode. All audio API calls (play sound, set volume, stop music) become silent no-ops that return `Ok(())`. The `DegradationManager` is notified:

```rust
pub fn init_audio(degradation: &mut DegradationManager) -> Box<dyn AudioBackend> {
    match AudioDevice::new() {
        Ok(device) => Box::new(RealAudioBackend::new(device)),
        Err(e) => {
            degradation.report_failure(Subsystem::Audio, &e);
            tracing::warn!("Audio unavailable, continuing without sound: {e}");
            Box::new(NullAudioBackend)
        }
    }
}
```

The `NullAudioBackend` implements the same `AudioBackend` trait but does nothing. This avoids conditional checks throughout the codebase -- systems call `audio.play("explosion.ogg")` and it silently succeeds regardless of whether audio is functional.

#### Network Disconnection

When the network connection drops during a multiplayer session, the engine transitions to an offline mode:

1. The `DegradationManager` marks `Subsystem::Network` as degraded.
2. Entity replication is paused; no new server state is applied.
3. The client continues simulating locally with the last known state.
4. A UI banner informs the player: "Connection lost. Playing offline."
5. The network system periodically attempts to reconnect.
6. On successful reconnect, `report_recovery` is called, replication resumes, and the server reconciles the client state.

```rust
pub fn handle_network_tick(
    connection: &mut Connection,
    degradation: &mut DegradationManager,
) {
    match connection.poll() {
        Ok(messages) => { /* process messages */ }
        Err(e) if e.is_disconnect() => {
            degradation.report_failure(Subsystem::Network, &e);
            connection.begin_reconnect();
        }
        Err(e) => {
            tracing::warn!("Network error (non-fatal): {e}");
        }
    }

    // If degraded, attempt periodic reconnection
    if degradation.is_degraded(Subsystem::Network) {
        if connection.try_reconnect().is_ok() {
            degradation.report_recovery(Subsystem::Network);
        }
    }
}
```

### Integration with Game Loop

The game loop checks `DegradationManager` each tick. If a critical system has failed, it initiates a graceful shutdown with an error dialog rather than panicking:

```rust
pub fn game_loop(degradation: &mut DegradationManager) {
    loop {
        degradation.tick();

        // Check if any critical subsystem has failed
        for (subsystem, status) in degradation.all_degraded() {
            if degradation.criticalities[&subsystem] == Criticality::Critical {
                tracing::error!("Critical subsystem {subsystem:?} has failed, shutting down");
                show_error_dialog("A critical engine component has failed. The engine must shut down.");
                return;
            }
        }

        // Run systems, skipping degraded optional systems
        // ...
    }
}
```

### UI Reporting

The debug UI panel displays the status of all subsystems. Degraded subsystems are shown in yellow (important) or grey (optional) with the duration of degradation. This gives developers and players immediate visibility into what is and is not working.

## Outcome

The engine continues running when non-critical subsystems fail. Audio failures result in silent operation. Shader failures produce magenta fallback rendering. Network disconnections transition to offline play with automatic reconnection. A `DegradationManager` resource tracks the health of every subsystem, distinguishing between critical, important, and optional tiers. The debug UI shows subsystem health at a glance. Developers can work on one subsystem even when another is broken, because the engine no longer crashes on non-critical initialization failures.

## Demo Integration

**Demo crate:** `nebula-demo`

If shadow map creation fails, shadows are disabled and the demo continues with a warning banner: "Shadows disabled: GPU texture limit reached." The game remains playable.

## Crates & Dependencies

- **`tracing = "0.1"`** -- Logging subsystem failures and recovery events at appropriate severity levels (`error` for failures, `info` for recoveries, `warn` for transient issues).
- **`thiserror = "2"`** -- Domain-specific error types that are passed to `DegradationManager::report_failure()` for structured error reporting.

No additional external crates are required. The `DegradationManager` is pure Rust with standard library types. The fallback audio backend uses a trait object pattern, and the fallback shader is compiled from a hardcoded WGSL string using the existing `wgpu` dependency.

## Unit Tests

- **`test_shader_failure_uses_fallback`** -- Construct a `DegradationManager`. Simulate a shader compilation failure by calling `report_failure(Subsystem::Render, &RenderError::ShaderCompilation { .. })`. Assert that `is_degraded(Subsystem::Render)` returns `true`. Verify that `report_failure` returns `true` (critical subsystem), confirming the engine knows this is serious. Separately, test the fallback path: create a function that returns either the compiled shader or a fallback, trigger an error, and assert the fallback is returned.

- **`test_audio_failure_does_not_crash`** -- Construct a `DegradationManager`. Call `report_failure(Subsystem::Audio, &err)` with a mock audio initialization error. Assert that `report_failure` returns `false` (optional subsystem, engine should not shut down). Assert `is_degraded(Subsystem::Audio)` returns `true`. Assert `is_healthy(Subsystem::Audio)` returns `false`.

- **`test_network_failure_transitions_to_offline`** -- Construct a `DegradationManager`. Call `report_failure(Subsystem::Network, &NetworkError::Disconnected { reason: "timeout".into() })`. Assert `is_degraded(Subsystem::Network)` is `true`. Assert `report_failure` returns `false` (optional, engine continues). Verify `all_degraded()` includes `Subsystem::Network`.

- **`test_degradation_state_is_queryable`** -- Register and degrade multiple subsystems. Call `all_degraded()` and assert it returns exactly the subsystems that were reported as failed. Call `status()` for each subsystem and verify it returns the correct `SubsystemStatus` variant. Verify healthy subsystems return `SubsystemStatus::Healthy`.

- **`test_recovery_restores_normal_state`** -- Degrade `Subsystem::Network` at tick 10. Advance ticks to 50. Call `report_recovery(Subsystem::Network)`. Assert `is_healthy(Subsystem::Network)` returns `true`. Assert `is_degraded(Subsystem::Network)` returns `false`. Assert the status is `SubsystemStatus::Recovered { degraded_for_ticks: 40 }`.

- **`test_critical_failure_signals_shutdown`** -- Call `report_failure(Subsystem::Render, &err)`. Assert the return value is `true`, indicating the engine should shut down. Contrast with `report_failure(Subsystem::Audio, &err)` which returns `false`.

- **`test_tick_advances_current_tick`** -- Create a `DegradationManager`. Call `tick()` 100 times. Then degrade a subsystem and verify the `since_tick` in the `Degraded` status is 100.

- **`test_multiple_subsystems_degraded_simultaneously`** -- Degrade `Audio`, `Network`, and `Particles`. Assert `all_degraded().len()` is 3. Assert each individual subsystem's status is `Degraded`. Assert `Render` is still `Healthy`.
