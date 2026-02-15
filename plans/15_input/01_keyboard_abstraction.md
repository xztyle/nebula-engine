# Keyboard Abstraction

## Problem

Winit 0.30 exposes raw keyboard events through its event loop, but consuming these directly throughout the engine couples gameplay code to windowing-library internals and makes it difficult to reason about per-frame key state. Game systems need a simple, frame-coherent API that answers three questions for any key: is it held right now, was it just pressed this frame, and was it just released this frame. The API must use physical key codes (scan codes) so that WASD movement works identically regardless of the user's keyboard layout (QWERTY, AZERTY, Dvorak, etc.).

## Solution

Introduce a `KeyboardState` struct that lives as a global resource in the ECS world and is updated once per frame from winit `WindowEvent::KeyboardInput` events.

```rust
use std::collections::HashSet;
use winit::keyboard::PhysicalKey;

pub struct KeyboardState {
    pressed: HashSet<PhysicalKey>,
    just_pressed: HashSet<PhysicalKey>,
    just_released: HashSet<PhysicalKey>,
}
```

### Update flow

1. **Event collection** -- During the winit event loop poll, every `KeyboardInput` event is forwarded to `KeyboardState::process_event(&mut self, event)`.
   - On `ElementState::Pressed` (and not a repeat): insert the key into `pressed` and `just_pressed`.
   - On `ElementState::Released`: remove the key from `pressed` and insert into `just_released`.
2. **Query** -- Game systems read state through the public API:
   - `is_pressed(key: PhysicalKey) -> bool` -- true while the key is held.
   - `just_pressed(key: PhysicalKey) -> bool` -- true only during the frame the key transitioned from released to pressed.
   - `just_released(key: PhysicalKey) -> bool` -- true only during the frame the key transitioned from pressed to released.
3. **End-of-frame clear** -- At the very end of the frame (after all systems have run), call `KeyboardState::clear_transients(&mut self)` which drains `just_pressed` and `just_released`.

Physical key codes (`PhysicalKey::Code(KeyCode::KeyW)`, etc.) are used everywhere so that the "W" position on QWERTY is the same physical scan code on AZERTY, ensuring layout-independent gameplay controls. Logical / character-based input is handled separately in the text-input context (Story 05).

## Outcome

A `keyboard.rs` module in `crates/nebula_input/src/` exporting `KeyboardState` with its public query methods, plus an internal `process_event` method and `clear_transients` method. The struct is registered as an ECS resource and integrated into the winit event loop adapter.

## Demo Integration

**Demo crate:** `nebula-demo`

WASD keys move the camera forward/back/left/right relative to the current facing direction. The demo transitions from passive observer to player-controlled.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| winit | 0.30 | Source of `KeyboardInput` events, `PhysicalKey`, `KeyCode` types |
| serde | 1.0 | Optional `Serialize`/`Deserialize` derives for snapshotting state in tests |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_initial_state_no_keys_pressed` | Construct a fresh `KeyboardState` and query several keys. | `is_pressed`, `just_pressed`, and `just_released` all return `false` for every key tested. |
| `test_press_event_sets_pressed` | Feed a `Pressed` event for `KeyCode::KeyW`. | `is_pressed(KeyW)` returns `true`; `just_pressed(KeyW)` returns `true`. |
| `test_release_clears_pressed` | Feed `Pressed` then `Released` for `KeyCode::KeyW` within the same frame. | `is_pressed(KeyW)` returns `false`; `just_released(KeyW)` returns `true`. |
| `test_just_pressed_true_for_one_frame_only` | Feed `Pressed` for `KeyCode::Space`, call `clear_transients`, then query. | After clear, `just_pressed(Space)` returns `false` while `is_pressed(Space)` remains `true`. |
| `test_just_released_true_for_one_frame_only` | Feed `Pressed`, `clear_transients`, then `Released`, then `clear_transients`, then query. | After the second clear, `just_released` returns `false` and `is_pressed` returns `false`. |
| `test_multiple_keys_tracked_independently` | Press `KeyW` and `KeyD`, release only `KeyW`. | `is_pressed(KeyW)` is `false`, `is_pressed(KeyD)` is `true`, `just_released(KeyW)` is `true`, `just_pressed(KeyD)` is `true`. |
| `test_repeat_events_ignored` | Feed two consecutive `Pressed` events for `KeyCode::KeyA` (second is a repeat). | `just_pressed(KeyA)` is `true` only once; no double-insert occurs. |
