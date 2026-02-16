//! Input context stack for managing multiple input modes (gameplay, menu, chat, etc.).
//!
//! Each [`InputContext`] specifies its own [`InputMap`], [`CursorMode`], and flags
//! controlling whether it consumes all input or forwards text events.
//! [`InputContextStack`] manages a stack of contexts; the topmost context determines
//! active bindings and cursor behavior.

use crate::action_map::{ActionResolver, ActionState, InputMap};
use crate::gamepad::GamepadState;
use crate::keyboard::KeyboardState;
use crate::mouse::MouseState;
use serde::{Deserialize, Serialize};

/// Whether the cursor is captured (FPS-style) or free (menu-style).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CursorMode {
    /// Hidden, centered, raw motion — typical for FPS gameplay.
    Captured,
    /// Visible, normal cursor — typical for menus and UI.
    Free,
}

/// A named input context with its own bindings, cursor mode, and behavior flags.
#[derive(Debug, Clone)]
pub struct InputContext {
    /// Human-readable name (e.g., `"gameplay"`, `"menu"`, `"chat"`).
    pub name: &'static str,
    /// The action bindings active in this context.
    pub input_map: InputMap,
    /// Cursor grab/visibility mode for this context.
    pub cursor_mode: CursorMode,
    /// If true, no contexts below this one receive input.
    pub consumes_input: bool,
    /// If true, raw key events are forwarded as text characters
    /// (for chat / console input).
    pub text_input: bool,
}

/// A stack of [`InputContext`] values.
///
/// The topmost context is the "active" one and determines cursor mode and primary bindings.
/// This is an ECS resource.
#[derive(Debug)]
pub struct InputContextStack {
    stack: Vec<InputContext>,
}

impl InputContextStack {
    /// Create a new stack with an initial gameplay context.
    #[must_use]
    pub fn new(initial: InputContext) -> Self {
        Self {
            stack: vec![initial],
        }
    }

    /// Push a new context onto the top of the stack.
    pub fn push_context(&mut self, ctx: InputContext) {
        self.stack.push(ctx);
    }

    /// Pop the topmost context. If only one context remains, this is a no-op.
    pub fn pop_context(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    /// Returns the active (topmost) context.
    ///
    /// # Panics
    /// The stack is guaranteed to always have at least one context.
    #[must_use]
    pub fn active_context(&self) -> &InputContext {
        self.stack.last().expect("InputContextStack is never empty")
    }

    /// Returns the number of contexts on the stack.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Resolve actions respecting the context stack.
    ///
    /// If the top context has `consumes_input: true`, only its bindings are evaluated.
    /// Otherwise, contexts are evaluated top-to-bottom until a consuming context is hit.
    /// When the active context has `text_input: true`, keyboard bindings are skipped
    /// (only mouse/gamepad bindings are resolved).
    pub fn resolve(
        &self,
        keyboard: &KeyboardState,
        mouse: &MouseState,
        gamepad: Option<&GamepadState>,
        state: &mut ActionState,
    ) {
        // Shift previous values.
        state.begin_frame();

        for ctx in self.stack.iter().rev() {
            if ctx.text_input {
                // In text-input mode, skip keyboard-based action resolution entirely.
                // Only resolve mouse/gamepad bindings from this context.
                ActionResolver::resolve_partial(
                    &ctx.input_map,
                    None, // no keyboard
                    mouse,
                    gamepad,
                    state,
                );
            } else {
                ActionResolver::resolve_partial(
                    &ctx.input_map,
                    Some(keyboard),
                    mouse,
                    gamepad,
                    state,
                );
            }

            if ctx.consumes_input {
                break;
            }
        }
    }
}

/// Accumulates text input characters for contexts with `text_input: true`.
///
/// Systems that need typed text (chat, console) read from this buffer each frame.
#[derive(Debug, Default, Clone)]
pub struct TextInputBuffer {
    /// Characters received this frame.
    chars: Vec<char>,
}

impl TextInputBuffer {
    /// Create a new empty text input buffer.
    #[must_use]
    pub fn new() -> Self {
        Self { chars: Vec::new() }
    }

    /// Push a character into the buffer (called by the window event handler).
    pub fn push(&mut self, ch: char) {
        self.chars.push(ch);
    }

    /// Read all characters accumulated this frame.
    #[must_use]
    pub fn chars(&self) -> &[char] {
        &self.chars
    }

    /// Clear the buffer (call at the end of each frame).
    pub fn clear(&mut self) {
        self.chars.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_map::{Action, InputBinding};
    use winit::event::ElementState;
    use winit::keyboard::{KeyCode, PhysicalKey};

    /// Helper: create a gameplay context with default FPS bindings and captured cursor.
    fn gameplay_context() -> InputContext {
        InputContext {
            name: "gameplay",
            input_map: InputMap::default_fps(),
            cursor_mode: CursorMode::Captured,
            consumes_input: true,
            text_input: false,
        }
    }

    /// Helper: create a menu context with Free cursor and consumes_input.
    fn menu_context() -> InputContext {
        let mut map = InputMap::new();
        map.set_bindings(Action::Pause, vec![InputBinding::Key(KeyCode::Escape)]);
        InputContext {
            name: "menu",
            input_map: map,
            cursor_mode: CursorMode::Free,
            consumes_input: true,
            text_input: false,
        }
    }

    /// Helper: create a chat context with text_input enabled.
    fn chat_context() -> InputContext {
        InputContext {
            name: "chat",
            input_map: InputMap::new(),
            cursor_mode: CursorMode::Free,
            consumes_input: true,
            text_input: true,
        }
    }

    /// Helper: create a debug overlay that does NOT consume input.
    fn debug_overlay_context() -> InputContext {
        let mut map = InputMap::new();
        map.set_bindings(Action::Interact, vec![InputBinding::Key(KeyCode::KeyF)]);
        InputContext {
            name: "debug_overlay",
            input_map: map,
            cursor_mode: CursorMode::Captured,
            consumes_input: false,
            text_input: false,
        }
    }

    /// Helper: press a key on a keyboard state.
    fn press_key(kb: &mut KeyboardState, code: KeyCode) {
        kb.process_raw(crate::keyboard::RawKeyEvent {
            key: PhysicalKey::Code(code),
            state: ElementState::Pressed,
            repeat: false,
        });
    }

    #[test]
    fn test_default_context_is_gameplay() {
        let stack = InputContextStack::new(gameplay_context());
        assert_eq!(stack.active_context().name, "gameplay");
        assert_eq!(stack.active_context().cursor_mode, CursorMode::Captured);
    }

    #[test]
    fn test_pushing_menu_context_changes_bindings() {
        let mut stack = InputContextStack::new(gameplay_context());
        stack.push_context(menu_context());
        assert_eq!(stack.active_context().name, "menu");
        assert_eq!(stack.active_context().cursor_mode, CursorMode::Free);
    }

    #[test]
    fn test_popping_restores_gameplay() {
        let mut stack = InputContextStack::new(gameplay_context());
        stack.push_context(menu_context());
        stack.pop_context();
        assert_eq!(stack.active_context().name, "gameplay");
        assert_eq!(stack.active_context().cursor_mode, CursorMode::Captured);
    }

    #[test]
    fn test_text_input_context_captures_all_keys() {
        let mut stack = InputContextStack::new(gameplay_context());
        stack.push_context(chat_context());
        assert!(stack.active_context().text_input);

        // Press W — should NOT activate MoveForward because chat consumes + text_input
        let mut kb = KeyboardState::new();
        press_key(&mut kb, KeyCode::KeyW);
        let mouse = MouseState::new();
        let mut state = ActionState::new();
        stack.resolve(&kb, &mouse, None, &mut state);
        assert!(
            !state.is_action_active(Action::MoveForward),
            "Keyboard action should be bypassed in text_input context"
        );
    }

    #[test]
    fn test_contexts_stack_correctly() {
        let mut stack = InputContextStack::new(gameplay_context());
        assert_eq!(stack.depth(), 1);

        stack.push_context(menu_context());
        assert_eq!(stack.depth(), 2);

        stack.push_context(chat_context());
        assert_eq!(stack.depth(), 3);
        assert_eq!(stack.active_context().name, "chat");

        stack.pop_context();
        assert_eq!(stack.depth(), 2);
        assert_eq!(stack.active_context().name, "menu");

        stack.pop_context();
        assert_eq!(stack.depth(), 1);
        assert_eq!(stack.active_context().name, "gameplay");
    }

    #[test]
    fn test_consumes_input_blocks_lower_contexts() {
        let mut stack = InputContextStack::new(gameplay_context());
        stack.push_context(menu_context()); // consumes_input: true

        // Press W — gameplay's MoveForward should NOT fire because menu consumes all input.
        let mut kb = KeyboardState::new();
        press_key(&mut kb, KeyCode::KeyW);
        let mouse = MouseState::new();
        let mut state = ActionState::new();
        stack.resolve(&kb, &mouse, None, &mut state);
        assert!(
            !state.is_action_active(Action::MoveForward),
            "Gameplay action should be blocked by consuming menu context"
        );

        // But Escape (bound in menu) should work.
        press_key(&mut kb, KeyCode::Escape);
        stack.resolve(&kb, &mouse, None, &mut state);
        assert!(
            state.is_action_active(Action::Pause),
            "Menu's Pause action should be active"
        );
    }

    #[test]
    fn test_non_consuming_overlay_passes_through() {
        let mut stack = InputContextStack::new(gameplay_context());
        stack.push_context(debug_overlay_context()); // consumes_input: false

        // Press W — should activate MoveForward from gameplay (passthrough).
        let mut kb = KeyboardState::new();
        press_key(&mut kb, KeyCode::KeyW);
        let mouse = MouseState::new();
        let mut state = ActionState::new();
        stack.resolve(&kb, &mouse, None, &mut state);
        assert!(
            state.is_action_active(Action::MoveForward),
            "Gameplay actions should pass through non-consuming overlay"
        );

        // Press F — should activate Interact from overlay.
        press_key(&mut kb, KeyCode::KeyF);
        stack.resolve(&kb, &mouse, None, &mut state);
        assert!(
            state.is_action_active(Action::Interact),
            "Overlay's Interact action should be active"
        );
    }

    #[test]
    fn test_pop_on_single_context_is_noop() {
        let mut stack = InputContextStack::new(gameplay_context());
        stack.pop_context();
        assert_eq!(stack.depth(), 1);
        assert_eq!(stack.active_context().name, "gameplay");
    }
}
