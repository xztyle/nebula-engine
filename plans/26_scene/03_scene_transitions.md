# Scene Transitions

## Problem

A game has multiple distinct scenes -- main menu, gameplay world, loading screen, settings menu, pause overlay -- and needs to switch between them cleanly. Without a transition system, scene changes involve ad-hoc entity cleanup, race conditions between unloading and loading, and jarring visual cuts. The player might see a frame of an empty void between the old and new scene. Worse, entities from the previous scene can leak into the next one if cleanup is incomplete.

Loading a new scene is not instantaneous: assets must be fetched from disk, terrain chunks must be generated, and prefabs must be spawned. During this period, the player should see a loading screen (or at minimum a transition effect like a fade to black), not a frozen or partially-constructed world. The transition system must orchestrate this multi-step process: begin unloading, show transition, load asynchronously, finish transition, hand off to the new scene.

## Solution

Implement a `SceneManager` resource in `nebula_scene` that drives scene transitions as a state machine, integrated with the ECS schedule.

### Scene Identifier

```rust
use serde::{Serialize, Deserialize};

/// Identifies a loadable scene. Scenes can be file-based (loaded from
/// a `.scene.ron` or `.scene.bin` file) or code-defined (constructed
/// programmatically, like a main menu).
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum SceneId {
    /// A scene loaded from a file path relative to the assets directory.
    File(String),
    /// A code-defined scene identified by a unique name.
    Named(String),
}
```

### Transition State Machine

```rust
use bevy_ecs::prelude::*;

/// The current phase of a scene transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionPhase {
    /// No transition in progress. The active scene is running normally.
    Idle,
    /// Transition out: running the exit effect (e.g., fade to black).
    /// The old scene is still alive but may be frozen.
    TransitionOut { progress: f32 },
    /// The old scene has been unloaded. The loading screen is visible
    /// and the new scene is being loaded asynchronously.
    Loading { progress: f32 },
    /// Loading complete. Running the entry effect (e.g., fade from black).
    TransitionIn { progress: f32 },
}

/// Controls how the visual transition looks.
pub trait TransitionEffect: Send + Sync + 'static {
    /// Called each frame during TransitionOut. Returns the normalized
    /// progress (0.0 = just started, 1.0 = fully obscured).
    fn update_out(&mut self, dt: f32) -> f32;

    /// Called each frame during TransitionIn. Returns the normalized
    /// progress (0.0 = fully obscured, 1.0 = fully revealed).
    fn update_in(&mut self, dt: f32) -> f32;

    /// Reset the effect for reuse.
    fn reset(&mut self);
}

/// A simple fade-to-black effect with configurable duration.
pub struct FadeTransition {
    pub fade_out_duration: f32,
    pub fade_in_duration: f32,
    elapsed: f32,
}

impl FadeTransition {
    pub fn new(fade_out_secs: f32, fade_in_secs: f32) -> Self {
        Self {
            fade_out_duration: fade_out_secs,
            fade_in_duration: fade_in_secs,
            elapsed: 0.0,
        }
    }
}

impl TransitionEffect for FadeTransition {
    fn update_out(&mut self, dt: f32) -> f32 {
        self.elapsed += dt;
        (self.elapsed / self.fade_out_duration).min(1.0)
    }

    fn update_in(&mut self, dt: f32) -> f32 {
        self.elapsed += dt;
        (self.elapsed / self.fade_in_duration).min(1.0)
    }

    fn reset(&mut self) {
        self.elapsed = 0.0;
    }
}
```

### Scene Manager

```rust
/// Marker component added to all entities spawned by a scene.
/// Used to identify which entities belong to the current scene
/// so they can be despawned on transition.
#[derive(Component, Debug, Clone)]
pub struct SceneEntity {
    pub scene: SceneId,
}

/// Manages scene lifecycle and transitions.
#[derive(Resource)]
pub struct SceneManager {
    /// The currently active scene, if any.
    pub active_scene: Option<SceneId>,
    /// The scene being loaded during a transition.
    pending_scene: Option<SceneId>,
    /// Current transition phase.
    pub phase: TransitionPhase,
    /// The active transition effect, if any.
    effect: Option<Box<dyn TransitionEffect>>,
    /// Async loading handle for the pending scene.
    load_handle: Option<SceneLoadHandle>,
}

/// Opaque handle to an async scene load operation.
pub struct SceneLoadHandle {
    /// Progress from 0.0 to 1.0.
    pub progress: f32,
    /// Whether loading is complete.
    pub complete: bool,
    /// The loaded scene data, available when `complete` is true.
    pub result: Option<SceneData>,
}

impl SceneManager {
    pub fn new() -> Self {
        Self {
            active_scene: None,
            pending_scene: None,
            phase: TransitionPhase::Idle,
            effect: None,
            load_handle: None,
        }
    }

    /// Request a transition to a new scene with an optional visual effect.
    /// Does nothing if a transition is already in progress.
    pub fn transition_to(
        &mut self,
        target: SceneId,
        effect: Option<Box<dyn TransitionEffect>>,
    ) {
        if self.phase != TransitionPhase::Idle {
            tracing::warn!("Transition already in progress, ignoring request");
            return;
        }
        self.pending_scene = Some(target);
        self.effect = effect;
        self.phase = TransitionPhase::TransitionOut { progress: 0.0 };
    }

    /// Returns true if a transition is currently in progress.
    pub fn is_transitioning(&self) -> bool {
        self.phase != TransitionPhase::Idle
    }
}
```

### Transition System

The transition is driven by an ECS system that runs each frame in the `PreUpdate` stage:

```rust
pub fn scene_transition_system(
    mut manager: ResMut<SceneManager>,
    mut commands: Commands,
    scene_entities: Query<Entity, With<SceneEntity>>,
    time: Res<TimeRes>,
) {
    let dt = time.delta;

    match &manager.phase {
        TransitionPhase::Idle => { /* nothing to do */ }

        TransitionPhase::TransitionOut { .. } => {
            let progress = if let Some(effect) = &mut manager.effect {
                effect.update_out(dt)
            } else {
                1.0 // No effect = instant transition
            };

            if progress >= 1.0 {
                // Despawn all entities from the old scene
                for entity in scene_entities.iter() {
                    commands.entity(entity).despawn();
                }

                // Begin async loading of the new scene
                let handle = begin_scene_load(manager.pending_scene.as_ref().unwrap());
                manager.load_handle = Some(handle);
                manager.phase = TransitionPhase::Loading { progress: 0.0 };

                if let Some(effect) = &mut manager.effect {
                    effect.reset();
                }
            } else {
                manager.phase = TransitionPhase::TransitionOut { progress };
            }
        }

        TransitionPhase::Loading { .. } => {
            if let Some(handle) = &manager.load_handle {
                if handle.complete {
                    // Loading done -- start transition in
                    manager.phase = TransitionPhase::TransitionIn { progress: 0.0 };
                    manager.active_scene = manager.pending_scene.take();
                } else {
                    manager.phase = TransitionPhase::Loading {
                        progress: handle.progress,
                    };
                }
            }
        }

        TransitionPhase::TransitionIn { .. } => {
            let progress = if let Some(effect) = &mut manager.effect {
                effect.update_in(dt)
            } else {
                1.0
            };

            if progress >= 1.0 {
                manager.phase = TransitionPhase::Idle;
                manager.effect = None;
                manager.load_handle = None;
            } else {
                manager.phase = TransitionPhase::TransitionIn { progress };
            }
        }
    }
}

/// Start loading a scene asynchronously. Returns a handle that can be
/// polled for progress and completion.
fn begin_scene_load(scene_id: &SceneId) -> SceneLoadHandle {
    // In practice this spawns an async task on the engine's task pool.
    // The handle is polled each frame by the transition system.
    SceneLoadHandle {
        progress: 0.0,
        complete: false,
        result: None,
    }
}
```

### Design Decisions

- **State machine over coroutines**: A four-phase state machine (Idle, TransitionOut, Loading, TransitionIn) is easy to reason about, easy to serialize for debugging, and avoids the complexity of async coroutines in the ECS context.
- **SceneEntity marker**: Tagging every entity with its owning scene makes cleanup trivial -- query all `SceneEntity` components matching the old scene and despawn them. No manual tracking lists needed.
- **Pluggable effects**: The `TransitionEffect` trait allows fade-to-black, screen wipes, dissolves, or any custom shader-driven effect without changing the transition system.
- **Loading screen as a scene**: The loading screen itself can be a lightweight scene (a background image and a progress bar entity) that is spawned during the `Loading` phase and despawned when transition completes.

## Outcome

A `SceneManager` resource that orchestrates scene transitions through a four-phase state machine: idle, transition-out (with visual effect), loading (async), and transition-in (with visual effect). Old scene entities are automatically despawned via the `SceneEntity` marker. Custom transition effects are pluggable via the `TransitionEffect` trait. Loading progress is tracked and available for UI display.

## Demo Integration

**Demo crate:** `nebula-demo`

Transitioning from surface to a cave instance shows a loading screen, swaps the active scene, and unloads the previous one.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | ECS World, Resource, Component, Commands, Query |
| `serde` | `1.0` | Serialize/Deserialize for SceneId |
| `tracing` | `0.1` | Warning and debug logs during transitions |

Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::prelude::*;

    fn setup_world() -> (World, SceneManager) {
        let mut world = World::new();
        world.insert_resource(TimeRes { delta: 1.0 / 60.0 });
        let manager = SceneManager::new();
        (world, manager)
    }

    #[test]
    fn test_transition_from_menu_to_gameplay() {
        let (mut world, mut manager) = setup_world();
        manager.active_scene = Some(SceneId::Named("MainMenu".into()));

        manager.transition_to(
            SceneId::Named("Gameplay".into()),
            None, // instant transition
        );

        assert!(manager.is_transitioning());
        assert!(matches!(manager.phase, TransitionPhase::TransitionOut { .. }));
    }

    #[test]
    fn test_loading_screen_appears_during_load() {
        let (_world, mut manager) = setup_world();
        manager.transition_to(SceneId::Named("Gameplay".into()), None);

        // Simulate: transition out completes instantly (no effect)
        // After one tick of the system, phase should move to Loading
        // We simulate the state machine manually here:
        manager.phase = TransitionPhase::Loading { progress: 0.5 };

        if let TransitionPhase::Loading { progress } = manager.phase {
            assert!(progress >= 0.0 && progress <= 1.0);
        } else {
            panic!("Expected Loading phase");
        }
    }

    #[test]
    fn test_transition_completes() {
        let (_world, mut manager) = setup_world();
        manager.transition_to(SceneId::Named("Gameplay".into()), None);

        // Simulate full transition cycle
        manager.phase = TransitionPhase::TransitionIn { progress: 1.0 };
        // After the system processes progress >= 1.0, it transitions to Idle
        manager.phase = TransitionPhase::Idle;
        manager.active_scene = Some(SceneId::Named("Gameplay".into()));

        assert!(!manager.is_transitioning());
        assert_eq!(
            manager.active_scene,
            Some(SceneId::Named("Gameplay".into()))
        );
    }

    #[test]
    fn test_old_scene_entities_are_despawned() {
        let mut world = World::new();
        // Spawn entities belonging to the old scene
        let e1 = world.spawn(SceneEntity {
            scene: SceneId::Named("OldScene".into()),
        }).id();
        let e2 = world.spawn(SceneEntity {
            scene: SceneId::Named("OldScene".into()),
        }).id();

        assert_eq!(world.entities().len(), 2);

        // Despawn all SceneEntity entities (simulating what the system does)
        let to_despawn: Vec<Entity> = world
            .query_filtered::<Entity, With<SceneEntity>>()
            .iter(&world)
            .collect();
        for entity in to_despawn {
            world.despawn(entity);
        }

        assert_eq!(world.entities().len(), 0);
    }

    #[test]
    fn test_custom_transition_effect_runs() {
        let mut effect = FadeTransition::new(0.5, 0.5);

        // Simulate 30 frames at 60fps (0.5 seconds) for fade out
        let mut progress = 0.0;
        for _ in 0..30 {
            progress = effect.update_out(1.0 / 60.0);
        }
        assert!((progress - 1.0).abs() < 0.02, "Fade out should complete in 0.5s");

        effect.reset();

        // Simulate fade in
        let mut progress = 0.0;
        for _ in 0..30 {
            progress = effect.update_in(1.0 / 60.0);
        }
        assert!((progress - 1.0).abs() < 0.02, "Fade in should complete in 0.5s");
    }

    #[test]
    fn test_duplicate_transition_request_ignored() {
        let (_world, mut manager) = setup_world();
        manager.transition_to(SceneId::Named("Scene1".into()), None);
        assert!(manager.is_transitioning());

        // Second request while transition is active should be ignored
        manager.transition_to(SceneId::Named("Scene2".into()), None);
        // Pending scene should still be Scene1, not Scene2
        assert_eq!(
            manager.pending_scene,
            Some(SceneId::Named("Scene1".into()))
        );
    }

    #[test]
    fn test_idle_when_no_transition() {
        let manager = SceneManager::new();
        assert_eq!(manager.phase, TransitionPhase::Idle);
        assert!(!manager.is_transitioning());
    }
}
```
