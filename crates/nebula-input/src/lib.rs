//! Input abstraction: keyboard, mouse, and gamepad mapped through configurable action-based keybindings.

pub mod action_map;
pub mod gamepad;
pub mod input_context;
pub mod keybindings;
pub mod keyboard;
pub mod mouse;

pub use action_map::{
    Action, ActionResolver, ActionState, GamepadAxisBinding, InputBinding, InputMap,
    MouseAxisBinding, MouseButtonBinding,
};
pub use gamepad::{GamepadAxes, GamepadManager, GamepadState, UnifiedButton};
pub use input_context::{CursorMode, InputContext, InputContextStack, TextInputBuffer};
pub use keybindings::{Conflict, Modifiers, RebindState};
pub use keyboard::{KeyboardState, RawKeyEvent};
pub use mouse::MouseState;
