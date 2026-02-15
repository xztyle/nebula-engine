# Input Context Switching

## Problem

A voxel game has many distinct interaction modes: exploring a cubesphere planet (WASD movement, mouse look, action bar), navigating a menu (mouse cursor, click to select), typing in a chat box (all keys consumed as text input), piloting a vehicle (throttle, pitch, roll mapped differently). These modes need completely different input bindings and cursor behavior. Without a structured context system, game code devolves into scattered `if in_menu { ... } else if in_chat { ... }` checks that are fragile and unmaintainable.

## Solution

Introduce an `InputContextStack` resource that manages a stack of named `InputContext` values. Each context specifies its own `InputMap` (action bindings), cursor mode (captured or free), and an optional flag indicating whether it consumes all input (blocking contexts below it).

```rust
pub struct InputContext {
    pub name: &'static str,
    pub input_map: InputMap,
    pub cursor_mode: CursorMode,
    /// If true, no contexts below this one receive input.
    pub consumes_input: bool,
    /// If true, raw key events are forwarded as text characters
    /// (for chat / console input).
    pub text_input: bool,
}

pub enum CursorMode {
    Captured,  // hidden, centered, raw motion
    Free,      // visible, normal cursor
}

pub struct InputContextStack {
    stack: Vec<InputContext>,
}
```

### Lifecycle

1. **Engine startup** -- Push the `"gameplay"` context with the default FPS `InputMap` and `CursorMode::Captured`.
2. **Opening a menu** -- `push_context(menu_context)` where `menu_context` has `CursorMode::Free`, `consumes_input: true`, and a menu-specific `InputMap` (click, escape to close). The cursor is released.
3. **Opening chat on top of menu** -- `push_context(chat_context)` where `text_input: true` and `consumes_input: true`. All keyboard events are routed to a text buffer instead of action resolution.
4. **Closing chat** -- `pop_context()` returns to the menu context, restoring its cursor mode and bindings.
5. **Closing menu** -- `pop_context()` returns to gameplay, re-capturing the cursor.

### Resolution

Each frame, the `ActionResolver` (Story 04) reads the `InputMap` from the **top** context on the stack. If `consumes_input` is true, only that context's map is evaluated. If false, contexts below it can also receive input (useful for overlays like an FPS counter that listens for a toggle key without blocking gameplay).

When `text_input` is true on the active context, `KeyboardState` still tracks physical keys, but an additional `TextInputBuffer` resource accumulates `WindowEvent::ReceivedCharacter`-equivalent events (winit 0.30 uses `Ime` events), which the chat/console system reads.

Cursor mode changes are applied to the winit `Window` whenever the top-of-stack changes.

## Outcome

An `input_context.rs` module in `crates/nebula_input/src/` exporting `InputContext`, `CursorMode`, and `InputContextStack`. The stack is an ECS resource. A `TextInputBuffer` resource is also provided for text-input contexts.

## Demo Integration

**Demo crate:** `nebula-demo`

When a menu is open (toggled with Escape), gameplay input is suppressed and menu navigation input activates. The input context switches cleanly between modes.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| winit | 0.30 | Cursor grab/visibility control, IME text input events |
| serde | 1.0 | Serialize context configurations for debugging and presets |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_default_context_is_gameplay` | Create an `InputContextStack` with the default gameplay context. | `stack.active_context().name` is `"gameplay"`; `cursor_mode` is `Captured`. |
| `test_pushing_menu_context_changes_bindings` | Push a `"menu"` context with `CursorMode::Free`. | `stack.active_context().name` is `"menu"`; `cursor_mode` is `Free`. |
| `test_popping_restores_gameplay` | Push `"menu"`, then pop. | `stack.active_context().name` is `"gameplay"`; `cursor_mode` is `Captured`. |
| `test_text_input_context_captures_all_keys` | Push a `"chat"` context with `text_input: true`. | `stack.active_context().text_input` is `true`; action resolution is bypassed for keyboard. |
| `test_contexts_stack_correctly` | Push `"menu"` then `"chat"`. Pop once. | Active is `"menu"`. Pop again. Active is `"gameplay"`. Stack depth changes from 3 to 2 to 1. |
| `test_consumes_input_blocks_lower_contexts` | Push `"gameplay"`, then push `"menu"` with `consumes_input: true`. | Only menu bindings are evaluated. Gameplay actions are not active even if their keys are pressed. |
| `test_non_consuming_overlay_passes_through` | Push `"gameplay"`, then push `"debug_overlay"` with `consumes_input: false`. | Both overlay and gameplay bindings are evaluated; gameplay actions still fire. |
| `test_pop_on_single_context_is_noop` | With only `"gameplay"` in the stack, call `pop_context()`. | Stack still has one entry; `active_context().name` is still `"gameplay"`. No panic. |
