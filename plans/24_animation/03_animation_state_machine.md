# Animation State Machine

## Problem

Real characters do not play a single animation clip in isolation. A humanoid character transitions between Idle, Walk, Run, Jump, and Attack animations based on gameplay state — velocity, input actions, timers, and flags. Hardcoding these transitions in gameplay code produces unmaintainable spaghetti: dozens of `if` branches checking overlapping conditions, no visual way to reason about the flow, and no clean way for designers to tweak transition timing without recompiling. Worse, abrupt clip switches cause visible popping — the character's pose snaps from one animation to another mid-stride. Smooth transitions require crossfading: blending two animation poses over a configurable duration.

The engine needs a data-driven animation state machine where states, transitions, and blending parameters are defined in an external asset file (RON format), loaded at runtime, and executed by a generic state machine evaluator that any animated entity can use.

## Solution

### State Machine Data Model

```rust
use std::collections::HashMap;

/// A named animation state that plays a specific clip.
#[derive(Debug, Clone)]
pub struct AnimationState {
    /// Unique name for this state (e.g., "Idle", "Walk", "Run").
    pub name: String,
    /// Index into the entity's AnimationClipLibrary.
    pub clip_index: usize,
    /// Playback speed multiplier for this state's clip.
    pub speed: f32,
    /// Whether the clip loops in this state.
    pub looping: bool,
}

/// A condition that must be satisfied to trigger a transition.
#[derive(Debug, Clone)]
pub enum TransitionCondition {
    /// A named float parameter exceeds a threshold (e.g., speed > 0.5).
    FloatGreaterThan { param: String, threshold: f32 },
    /// A named float parameter is below a threshold (e.g., speed < 0.1).
    FloatLessThan { param: String, threshold: f32 },
    /// A named boolean parameter is true (e.g., "is_jumping").
    BoolTrue { param: String },
    /// A named boolean parameter is false (e.g., "is_grounded").
    BoolFalse { param: String },
    /// A named trigger was activated this frame (consumed on read).
    Trigger { param: String },
    /// The current state's clip has finished playing (non-looping clips only).
    ClipFinished,
}

/// A transition between two states.
#[derive(Debug, Clone)]
pub struct StateTransition {
    /// Source state name.
    pub from: String,
    /// Destination state name.
    pub to: String,
    /// Duration of the crossfade blend in seconds.
    pub blend_duration: f32,
    /// All conditions must be true for the transition to fire (AND logic).
    pub conditions: Vec<TransitionCondition>,
    /// Priority when multiple transitions are valid (higher wins).
    pub priority: u8,
}

/// The complete state machine definition, loadable from RON.
#[derive(Debug, Clone)]
pub struct AnimationStateMachineDefinition {
    /// All states in the machine.
    pub states: Vec<AnimationState>,
    /// All transitions.
    pub transitions: Vec<StateTransition>,
    /// Name of the initial state.
    pub initial_state: String,
}
```

### RON Asset Format

State machines are defined in `.anim_fsm.ron` files:

```ron
AnimationStateMachineDefinition(
    initial_state: "Idle",
    states: [
        AnimationState(name: "Idle",   clip_index: 0, speed: 1.0, looping: true),
        AnimationState(name: "Walk",   clip_index: 1, speed: 1.0, looping: true),
        AnimationState(name: "Run",    clip_index: 2, speed: 1.0, looping: true),
        AnimationState(name: "Jump",   clip_index: 3, speed: 1.0, looping: false),
        AnimationState(name: "Attack", clip_index: 4, speed: 1.2, looping: false),
    ],
    transitions: [
        StateTransition(from: "Idle", to: "Walk", blend_duration: 0.2,
            conditions: [FloatGreaterThan(param: "speed", threshold: 0.5)], priority: 1),
        StateTransition(from: "Walk", to: "Run", blend_duration: 0.3,
            conditions: [FloatGreaterThan(param: "speed", threshold: 4.0)], priority: 1),
        StateTransition(from: "Walk", to: "Idle", blend_duration: 0.2,
            conditions: [FloatLessThan(param: "speed", threshold: 0.1)], priority: 1),
        StateTransition(from: "Run", to: "Walk", blend_duration: 0.3,
            conditions: [FloatLessThan(param: "speed", threshold: 4.0)], priority: 1),
        StateTransition(from: "Idle", to: "Jump", blend_duration: 0.1,
            conditions: [Trigger(param: "jump")], priority: 2),
        StateTransition(from: "Walk", to: "Jump", blend_duration: 0.1,
            conditions: [Trigger(param: "jump")], priority: 2),
        StateTransition(from: "Jump", to: "Idle", blend_duration: 0.15,
            conditions: [ClipFinished, BoolTrue(param: "is_grounded")], priority: 1),
        StateTransition(from: "Idle", to: "Attack", blend_duration: 0.1,
            conditions: [Trigger(param: "attack")], priority: 3),
        StateTransition(from: "Attack", to: "Idle", blend_duration: 0.2,
            conditions: [ClipFinished], priority: 1),
    ],
)
```

The `ron` crate deserializes this into the `AnimationStateMachineDefinition` struct. All types derive `serde::Deserialize`.

### Runtime State Machine

```rust
/// Runtime state for an active animation state machine, attached as an ECS component.
pub struct AnimationStateMachine {
    /// The definition this instance is running.
    pub definition: AnimationStateMachineDefinition,
    /// Current active state name.
    pub current_state: String,
    /// If a transition is in progress, the state being transitioned from.
    pub previous_state: Option<String>,
    /// Transition blend progress: 0.0 = fully previous, 1.0 = fully current.
    pub blend_weight: f32,
    /// Total blend duration of the active transition.
    pub blend_duration: f32,
    /// Named parameters set by gameplay code.
    pub float_params: HashMap<String, f32>,
    pub bool_params: HashMap<String, bool>,
    /// Triggers are consumed after evaluation (one-shot).
    pub triggers: HashMap<String, bool>,
}
```

### Evaluation System

The `animation_state_machine_system` runs before `animation_playback_system`:

1. **Evaluate transitions**. For the current state, collect all transitions whose `from` matches `current_state`. Check each transition's conditions against the parameter maps. If multiple transitions are valid, select the one with the highest priority. Consume any triggers that were checked.

2. **Start transition**. When a transition fires:
   - Set `previous_state = Some(old_current_state)`.
   - Set `current_state = transition.to`.
   - Set `blend_weight = 0.0`, `blend_duration = transition.blend_duration`.
   - Start the new state's clip from time 0 on the `AnimationPlayer`.

3. **Advance blend**. If `previous_state.is_some()`:
   - Advance `blend_weight += dt / blend_duration`.
   - If `blend_weight >= 1.0`, finalize: set `previous_state = None`, `blend_weight = 1.0`.

4. **Crossfade blending**. When a transition is active, both the previous and current clips are sampled at their respective playback times. The final pose for each joint is:
   ```rust
   let blended_translation = prev_translation.lerp(curr_translation, blend_weight);
   let blended_rotation = prev_rotation.slerp(curr_rotation, blend_weight);
   let blended_scale = prev_scale.lerp(curr_scale, blend_weight);
   ```
   This produces a smooth visual transition without popping.

5. **Write to AnimationPlayer**. The state machine sets the `AnimationPlayer`'s active clip index (or dual clip indices during crossfade) and lets the playback system handle the rest.

### Parameter API

Gameplay systems set parameters through the component:

```rust
impl AnimationStateMachine {
    pub fn set_float(&mut self, name: &str, value: f32) {
        self.float_params.insert(name.to_owned(), value);
    }

    pub fn set_bool(&mut self, name: &str, value: bool) {
        self.bool_params.insert(name.to_owned(), value);
    }

    pub fn fire_trigger(&mut self, name: &str) {
        self.triggers.insert(name.to_owned(), true);
    }
}
```

## Outcome

A data-driven `AnimationStateMachine` component that reads a RON-defined state graph, evaluates transition conditions against gameplay-set parameters, and drives crossfade blending between animation clips. Designers define state machines in `.anim_fsm.ron` files without touching Rust code. `cargo test -p nebula-animation` passes all state machine evaluation, transition, and blending tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Idle, walk, and run animations transition smoothly based on movement speed. Blending prevents snapping between states.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | `0.32` | `Vec3::lerp`, `Quat::slerp` for crossfade blending between poses |
| `ron` | `0.9` | Deserialize animation state machine definitions from `.anim_fsm.ron` files |
| `serde` | `1.0` | `Deserialize` derive for all state machine data types |
| `bevy_ecs` | `0.18` | `Component` derive, `Query`, `Res<Time>` for the evaluation system |
| `log` | `0.4` | Warn on missing parameters, undefined state references, zero-length blend durations |

All dependencies are declared in `[workspace.dependencies]` and consumed via `{ workspace = true }` in the `nebula-animation` crate's `Cargo.toml`. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a minimal state machine with Idle, Walk, and Run states.
    fn create_test_fsm() -> AnimationStateMachine {
        let def = AnimationStateMachineDefinition {
            initial_state: "Idle".into(),
            states: vec![
                AnimationState { name: "Idle".into(), clip_index: 0, speed: 1.0, looping: true },
                AnimationState { name: "Walk".into(), clip_index: 1, speed: 1.0, looping: true },
                AnimationState { name: "Run".into(),  clip_index: 2, speed: 1.0, looping: true },
            ],
            transitions: vec![
                StateTransition {
                    from: "Idle".into(), to: "Walk".into(), blend_duration: 0.2,
                    conditions: vec![TransitionCondition::FloatGreaterThan {
                        param: "speed".into(), threshold: 0.5,
                    }],
                    priority: 1,
                },
                StateTransition {
                    from: "Walk".into(), to: "Run".into(), blend_duration: 0.3,
                    conditions: vec![TransitionCondition::FloatGreaterThan {
                        param: "speed".into(), threshold: 4.0,
                    }],
                    priority: 1,
                },
                StateTransition {
                    from: "Walk".into(), to: "Idle".into(), blend_duration: 0.2,
                    conditions: vec![TransitionCondition::FloatLessThan {
                        param: "speed".into(), threshold: 0.1,
                    }],
                    priority: 1,
                },
                StateTransition {
                    from: "Run".into(), to: "Walk".into(), blend_duration: 0.3,
                    conditions: vec![TransitionCondition::FloatLessThan {
                        param: "speed".into(), threshold: 4.0,
                    }],
                    priority: 1,
                },
            ],
        };
        AnimationStateMachine::new(def)
    }

    /// Verify that the state machine starts in the initial state.
    #[test]
    fn test_initial_state_is_idle() {
        let fsm = create_test_fsm();
        assert_eq!(fsm.current_state, "Idle");
        assert!(fsm.previous_state.is_none());
    }

    /// Verify that setting speed > 0.5 triggers Idle -> Walk transition.
    #[test]
    fn test_speed_triggers_walk() {
        let mut fsm = create_test_fsm();
        fsm.set_float("speed", 2.0);
        fsm.evaluate(0.016); // one frame at ~60 FPS
        assert_eq!(fsm.current_state, "Walk");
    }

    /// Verify that Walk -> Run transition fires when speed exceeds threshold.
    #[test]
    fn test_walk_to_run_transition() {
        let mut fsm = create_test_fsm();

        // Idle -> Walk
        fsm.set_float("speed", 2.0);
        fsm.evaluate(0.016);
        assert_eq!(fsm.current_state, "Walk");

        // Complete the blend so the state machine is ready for the next transition.
        for _ in 0..20 {
            fsm.evaluate(0.016);
        }

        // Walk -> Run
        fsm.set_float("speed", 5.0);
        fsm.evaluate(0.016);
        assert_eq!(fsm.current_state, "Run");
    }

    /// Verify that a transition crossfades: blend_weight starts at 0 and increases to 1.
    #[test]
    fn test_transition_crossfades() {
        let mut fsm = create_test_fsm();
        fsm.set_float("speed", 2.0);
        fsm.evaluate(0.016);

        // Transition should be in progress.
        assert!(fsm.previous_state.is_some(), "should have a previous state during crossfade");
        assert!(
            fsm.blend_weight > 0.0 && fsm.blend_weight < 1.0,
            "blend_weight should be between 0 and 1 during crossfade, got {}",
            fsm.blend_weight
        );

        // Advance until blend completes (blend_duration = 0.2s).
        for _ in 0..20 {
            fsm.evaluate(0.016);
        }
        assert!(
            fsm.previous_state.is_none(),
            "crossfade should be complete after sufficient time"
        );
        assert!(
            (fsm.blend_weight - 1.0).abs() < 1e-5,
            "blend_weight should be 1.0 after crossfade completes"
        );
    }

    /// Verify that the state machine returns to Idle when speed drops below threshold.
    #[test]
    fn test_returns_to_idle_when_stopped() {
        let mut fsm = create_test_fsm();

        // Idle -> Walk
        fsm.set_float("speed", 2.0);
        fsm.evaluate(0.016);
        for _ in 0..20 {
            fsm.evaluate(0.016);
        }
        assert_eq!(fsm.current_state, "Walk");

        // Walk -> Idle
        fsm.set_float("speed", 0.0);
        fsm.evaluate(0.016);
        assert_eq!(fsm.current_state, "Idle");
    }
}
```
