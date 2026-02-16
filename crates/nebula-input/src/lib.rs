//! Input abstraction: keyboard, mouse, and gamepad mapped through configurable action-based keybindings.

pub mod keyboard;
pub mod mouse;

pub use keyboard::{KeyboardState, RawKeyEvent};
pub use mouse::MouseState;
