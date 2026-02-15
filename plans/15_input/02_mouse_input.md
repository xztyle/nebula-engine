# Mouse Input

## Problem

First-person and third-person cameras on cubesphere-voxel planets require smooth, frame-coherent mouse input covering position, delta movement, button states, and scroll wheel. When the player is in gameplay mode the cursor must be captured (hidden, centered, raw motion enabled) to provide infinite mouse-look without hitting screen edges. When the player opens a menu the cursor must be released and visible again. Winit 0.30 provides the raw events, but the engine needs a unified `MouseState` that game systems can query without touching the windowing layer.

## Solution

Define a `MouseState` struct that accumulates winit mouse events during the event-collection phase and exposes a clean query API.

```rust
use glam::Vec2;

pub struct MouseState {
    /// Current cursor position in window-logical coordinates.
    position: Vec2,
    /// Movement delta since last frame (pixels or raw units when captured).
    delta: Vec2,
    /// Button states indexed by MouseButton ordinal (Left=0 .. Other=4).
    buttons: [ButtonFrame; 5],
    /// Scroll wheel delta accumulated this frame (positive = scroll up).
    scroll: f32,
    /// Whether the cursor is currently captured for FPS-style look.
    captured: bool,
    /// Whether the cursor is inside the window.
    cursor_in_window: bool,
}

struct ButtonFrame {
    pressed: bool,
    just_pressed: bool,
    just_released: bool,
}
```

### Update flow

1. **`CursorMoved` event** -- Update `position`. Compute `delta` as `new_position - previous_position`. When captured, winit delivers `DeviceEvent::MouseMotion` with raw deltas instead; accumulate those into `delta` directly.
2. **`MouseInput` event** -- On `Pressed`: set `buttons[i].pressed = true`, `just_pressed = true`. On `Released`: set `pressed = false`, `just_released = true`.
3. **`MouseWheel` event** -- Accumulate `LineDelta` or `PixelDelta` (normalized) into `scroll`.
4. **`CursorEntered` / `CursorLeft`** -- Toggle `cursor_in_window`.
5. **Capture / release** -- `MouseState::set_captured(&mut self, window: &Window, captured: bool)` calls `window.set_cursor_grab(CursorGrabMode::Confined)` or `Locked` plus `window.set_cursor_visible(!captured)`. When captured, delta comes from raw `DeviceEvent::MouseMotion`.
6. **End-of-frame clear** -- Reset `delta` to zero, `scroll` to zero, and clear all `just_pressed` / `just_released` flags.

Public query methods:

- `position() -> Vec2`
- `delta() -> Vec2`
- `is_button_pressed(button: MouseButton) -> bool`
- `just_button_pressed(button: MouseButton) -> bool`
- `just_button_released(button: MouseButton) -> bool`
- `scroll() -> f32`
- `is_captured() -> bool`
- `is_cursor_in_window() -> bool`

## Outcome

A `mouse.rs` module in `crates/nebula_input/src/` exporting `MouseState` with the above API. Integrated into the winit event loop alongside `KeyboardState`.

## Demo Integration

**Demo crate:** `nebula-demo`

Mouse movement rotates the camera view. The mouse cursor is captured by the window for infinite look-around. Camera control feels natural and responsive.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| winit | 0.30 | Source of cursor, mouse button, scroll, and device motion events; cursor grab API |
| glam | (workspace) | `Vec2` for position and delta |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_position_updates_on_move` | Feed a `CursorMoved` event with position `(100.0, 200.0)`. | `position()` returns `Vec2::new(100.0, 200.0)`. |
| `test_delta_is_difference_between_frames` | Feed `CursorMoved(100, 200)`, clear frame, feed `CursorMoved(110, 195)`. | `delta()` returns `Vec2::new(10.0, -5.0)`. |
| `test_button_press_and_release_tracked` | Feed `MouseInput { button: Left, state: Pressed }`, then `Released`. | After press: `is_button_pressed(Left)` true, `just_button_pressed(Left)` true. After release: `is_button_pressed(Left)` false, `just_button_released(Left)` true. |
| `test_scroll_accumulates_within_frame` | Feed two `MouseWheel` events with deltas `1.0` and `0.5`. | `scroll()` returns `1.5`. |
| `test_scroll_resets_after_clear` | Feed scroll `1.0`, call `clear_transients`. | `scroll()` returns `0.0`. |
| `test_cursor_capture_sets_flag` | Call `set_captured(true)` (with a mock window). | `is_captured()` returns `true`. |
| `test_cursor_enter_leave` | Feed `CursorEntered`, then `CursorLeft`. | `is_cursor_in_window()` toggles from `true` to `false`. |
| `test_delta_resets_each_frame` | Feed a motion event, clear, query. | `delta()` returns `Vec2::ZERO` after clear. |
