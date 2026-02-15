# Gamepad Support

## Problem

Nebula Engine targets Linux, Windows, and macOS, and many players prefer gamepads for exploration and flight across cubesphere-voxel planets. Each OS has different gamepad APIs (XInput, evdev, IOKit) and different default button mappings. The engine needs a unified gamepad abstraction that normalizes axes and buttons, handles analog dead zones, and supports hot-plugging so players can connect or disconnect controllers at any time without crashing or stalling.

## Solution

Use the `gilrs` (Game Input Library for Rust) crate version 0.11, which already abstracts platform-specific gamepad APIs behind a single interface. Wrap gilrs into a Nebula-specific `GamepadState` resource.

```rust
use gilrs::{Gilrs, GamepadId, Axis, Button};
use std::collections::HashMap;

pub struct GamepadManager {
    gilrs: Gilrs,
    gamepads: HashMap<GamepadId, GamepadState>,
    /// Default deadzone threshold for analog sticks.
    deadzone: f32, // default 0.15
}

pub struct GamepadState {
    id: GamepadId,
    name: String,
    connected: bool,
    axes: GamepadAxes,
    buttons: HashMap<UnifiedButton, ButtonFrame>,
}

pub struct GamepadAxes {
    left_stick: Vec2,   // x: left(-1)..right(+1), y: down(-1)..up(+1)
    right_stick: Vec2,
    left_trigger: f32,  // 0.0 .. 1.0
    right_trigger: f32, // 0.0 .. 1.0
}
```

### Update flow

1. **Poll gilrs** -- Each frame, call `gilrs.next_event()` in a loop to drain all pending events.
2. **Connection events** -- `EventType::Connected`: insert a new `GamepadState` into the map with the gamepad's name and ID. `EventType::Disconnected`: mark `connected = false` (keep the entry so reconnection restores context).
3. **Axis events** -- Map `gilrs::Axis` values to the unified `GamepadAxes` fields. Apply deadzone: if `value.abs() < deadzone`, clamp to `0.0`. Rescale the remaining range from `[deadzone, 1.0]` to `[0.0, 1.0]` for a smooth response curve.
4. **Button events** -- Map to `UnifiedButton` enum (South/A, East/B, North/Y, West/X, DPad, Shoulders, Sticks, Start, Select). Track `pressed`, `just_pressed`, `just_released` per button.
5. **End-of-frame clear** -- Clear `just_pressed` / `just_released` for all buttons on all gamepads.

Public API:

- `GamepadManager::connected_gamepads() -> impl Iterator<Item = GamepadId>`
- `GamepadManager::gamepad(id) -> Option<&GamepadState>`
- `GamepadState::left_stick() -> Vec2`
- `GamepadState::right_stick() -> Vec2`
- `GamepadState::left_trigger() -> f32`
- `GamepadState::is_button_pressed(button) -> bool`
- `GamepadState::just_button_pressed(button) -> bool`
- `GamepadManager::set_deadzone(value: f32)`

The deadzone is configurable at runtime and can be persisted in the settings RON file (Story 06).

## Outcome

A `gamepad.rs` module in `crates/nebula_input/src/` exporting `GamepadManager` and `GamepadState`. The `GamepadManager` is registered as an ECS resource and polled once per frame before game systems run.

## Demo Integration

**Demo crate:** `nebula-demo`

A connected gamepad's left stick moves the camera, right stick rotates the view. The demo works with Xbox, PlayStation, and generic controllers out of the box.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| gilrs | 0.11 | Cross-platform gamepad input (evdev, XInput, IOKit) |
| glam | (workspace) | `Vec2` for stick axes |
| serde | 1.0 | Serialize deadzone and button-mapping preferences |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_gamepad_connection_detected` | Simulate a `Connected` event for a gamepad. | `connected_gamepads().count()` returns `1`; `gamepad(id).unwrap().connected` is `true`. |
| `test_axis_values_in_range` | Feed axis events with values `-1.0`, `0.0`, `1.0`. | `left_stick()` components are each within `[-1.0, 1.0]`. |
| `test_deadzone_filters_small_values` | Set deadzone to `0.15`, feed an axis event with value `0.10`. | `left_stick().x` returns `0.0`. |
| `test_deadzone_rescales_above_threshold` | Set deadzone to `0.15`, feed axis value `0.575`. | Rescaled value is approximately `0.5` (mapped from `[0.15, 1.0]` to `[0.0, 1.0]`). |
| `test_button_state_tracked` | Feed `ButtonPressed(South)` then `ButtonReleased(South)`. | After press: `is_button_pressed(South)` true. After release: false, `just_button_released(South)` true. |
| `test_disconnection_handled_gracefully` | Feed `Connected`, then `Disconnected`. | `gamepad(id).unwrap().connected` is `false`; `connected_gamepads().count()` is `0`; no panic. |
| `test_multiple_gamepads_supported` | Feed `Connected` for two different gamepad IDs. | `connected_gamepads().count()` returns `2`; each gamepad has independent state. |
| `test_custom_deadzone` | Set deadzone to `0.25`, feed axis value `0.20`. | Returns `0.0` (filtered). Feed `0.30`: returns non-zero rescaled value. |
