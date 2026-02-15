# Action Mapping System

## Problem

Hard-coding physical inputs (e.g., "if W is pressed, move forward") scatters input logic throughout game systems and makes rebinding impossible. Different input devices (keyboard, mouse, gamepad) produce fundamentally different data types (digital on/off vs. analog 0.0--1.0), yet a game action like "MoveForward" should work seamlessly with either a key press (returns 1.0 when held) or a stick axis (returns a continuous value). The engine needs an indirection layer that maps abstract game actions to one or more physical input sources.

## Solution

Introduce an `InputMap` resource that maps `Action` enum variants to lists of `InputBinding` values, and an `ActionState` resource that is recomputed each frame from the current `KeyboardState`, `MouseState`, and `GamepadState`.

```rust
use std::collections::HashMap;

#[derive(Clone, Copy, Hash, Eq, PartialEq)]
pub enum Action {
    MoveForward,
    MoveBack,
    MoveLeft,
    MoveRight,
    Jump,
    Crouch,
    Sprint,
    PrimaryAction,
    SecondaryAction,
    Interact,
    OpenInventory,
    Pause,
    // extensible via user-defined actions in the future
}

pub enum InputBinding {
    Key(PhysicalKey),
    MouseButton(MouseButton),
    MouseAxis(MouseAxisBinding),   // e.g., mouse-X for yaw
    GamepadButton(UnifiedButton),
    GamepadAxis(GamepadAxisBinding), // e.g., left-stick-y for forward
}

pub struct InputMap {
    bindings: HashMap<Action, Vec<InputBinding>>,
}

pub struct ActionState {
    values: HashMap<Action, f32>,
}
```

### Update flow

1. Each frame, after `KeyboardState`, `MouseState`, and `GamepadState` have been updated, the `ActionResolver` system iterates over every `(Action, Vec<InputBinding>)` entry in the `InputMap`.
2. For each binding, it reads the current value from the appropriate state resource:
   - `InputBinding::Key(key)` -> `1.0` if `keyboard.is_pressed(key)`, else `0.0`.
   - `InputBinding::MouseButton(btn)` -> `1.0` if pressed, else `0.0`.
   - `InputBinding::MouseAxis(axis)` -> raw delta value from `MouseState`.
   - `InputBinding::GamepadButton(btn)` -> `1.0` if pressed, else `0.0`.
   - `InputBinding::GamepadAxis(axis)` -> analog value from `GamepadState`.
3. Multiple bindings for the same action use **OR** logic for digital (max of values) and **sum** for analog axes, clamped to `[-1.0, 1.0]`.
4. The result is written into `ActionState`.

Public query API:

- `is_action_active(action: Action) -> bool` -- true if value is above a small threshold (0.001).
- `action_value(action: Action) -> f32` -- the analog value in `[-1.0, 1.0]`.
- `action_just_activated(action: Action) -> bool` -- edge-triggered, true only the frame value went from inactive to active.
- `action_just_deactivated(action: Action) -> bool` -- the reverse edge.

The `InputMap` can be modified at runtime (for rebinding, see Story 06) and is serializable to RON.

## Outcome

An `action_map.rs` module in `crates/nebula_input/src/` exporting `Action`, `InputBinding`, `InputMap`, `ActionState`, and the `ActionResolver` system. The default `InputMap` provides standard FPS-style bindings (WASD + mouse look + gamepad sticks).

## Demo Integration

**Demo crate:** `nebula-demo`

Raw keys are abstracted into semantic actions: `MoveForward`, `LookUp`, `Sprint`. Multiple keys/buttons can map to the same action. The demo queries actions, not raw keys.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| winit | 0.30 | `PhysicalKey`, `MouseButton` types referenced in bindings |
| gilrs | 0.11 | Gamepad button/axis types referenced in bindings |
| serde | 1.0 | Serialize/Deserialize `InputMap` and `Action` for config persistence |
| ron | 0.12 | RON format for human-readable input configuration files |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_action_bound_to_key_activates_on_press` | Bind `MoveForward` to `KeyW`, simulate `KeyW` pressed. | `is_action_active(MoveForward)` returns `true`; `action_value(MoveForward)` returns `1.0`. |
| `test_action_bound_to_gamepad_axis_returns_analog` | Bind `MoveForward` to left-stick-Y, set axis to `0.75`. | `action_value(MoveForward)` returns `0.75`. |
| `test_unbound_action_returns_false_and_zero` | Query `OpenInventory` with no binding set. | `is_action_active(OpenInventory)` false; `action_value(OpenInventory)` is `0.0`. |
| `test_multiple_bindings_or_logic` | Bind `Jump` to both `Space` key and gamepad `South` button. Press `Space` only. | `is_action_active(Jump)` returns `true`. |
| `test_multiple_bindings_both_active` | Bind `Jump` to `Space` and gamepad `South`. Press both. | `action_value(Jump)` is `1.0` (clamped, not `2.0`). |
| `test_action_map_modified_at_runtime` | Start with `Jump` bound to `Space`. Rebind to `KeyJ`. Press `KeyJ`. | `is_action_active(Jump)` returns `true`; pressing `Space` no longer activates `Jump`. |
| `test_action_just_activated_edge` | Bind `Jump` to `Space`. Frame 1: press `Space`. Frame 2: still held. | Frame 1: `action_just_activated(Jump)` true. Frame 2: `action_just_activated(Jump)` false. |
| `test_analog_sum_clamped` | Bind `MoveForward` to both `KeyW` (1.0) and left-stick-Y (0.8). Both active. | `action_value(MoveForward)` is `1.0` (clamped from 1.8). |
