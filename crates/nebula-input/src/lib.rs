//! Input abstraction: keyboard, mouse, and gamepad mapped through configurable action-based keybindings.

pub mod keyboard;

pub use keyboard::{KeyboardState, RawKeyEvent};
