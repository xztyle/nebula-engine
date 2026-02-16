//! Input abstraction: keyboard, mouse, and gamepad mapped through configurable action-based keybindings.

pub mod gamepad;
pub mod keyboard;
pub mod mouse;

pub use gamepad::{GamepadAxes, GamepadManager, GamepadState, UnifiedButton};
pub use keyboard::{KeyboardState, RawKeyEvent};
pub use mouse::MouseState;
