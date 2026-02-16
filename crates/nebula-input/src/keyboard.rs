//! Frame-coherent keyboard state tracker.
//!
//! [`KeyboardState`] accumulates winit [`KeyboardInput`] events during a frame
//! and answers three questions for any physical key: is it held, was it just
//! pressed this frame, and was it just released this frame.
//!
//! Physical key codes are used throughout so that WASD movement works
//! identically regardless of the user's keyboard layout.

use std::collections::HashSet;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::PhysicalKey;

/// Minimal description of a key event for processing.
#[derive(Debug, Clone, Copy)]
pub struct RawKeyEvent {
    /// The physical key involved.
    pub key: PhysicalKey,
    /// Whether the key was pressed or released.
    pub state: ElementState,
    /// Whether this is a repeat event.
    pub repeat: bool,
}

/// Tracks per-frame keyboard state using physical (scan-code) keys.
///
/// # Usage
///
/// 1. Forward every [`KeyEvent`] to [`process_event`](Self::process_event).
/// 2. Query state with [`is_pressed`](Self::is_pressed),
///    [`just_pressed`](Self::just_pressed), [`just_released`](Self::just_released).
/// 3. Call [`clear_transients`](Self::clear_transients) at the end of each frame.
#[derive(Debug, Clone)]
pub struct KeyboardState {
    pressed: HashSet<PhysicalKey>,
    just_pressed: HashSet<PhysicalKey>,
    just_released: HashSet<PhysicalKey>,
}

impl Default for KeyboardState {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyboardState {
    /// Creates a new `KeyboardState` with no keys pressed.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pressed: HashSet::new(),
            just_pressed: HashSet::new(),
            just_released: HashSet::new(),
        }
    }

    /// Processes a winit [`KeyEvent`], updating internal state.
    ///
    /// - **Pressed** (non-repeat): inserts into `pressed` and `just_pressed`.
    /// - **Released**: removes from `pressed`, inserts into `just_released`.
    /// - Repeat events are ignored.
    pub fn process_event(&mut self, event: &KeyEvent) {
        self.process_raw(RawKeyEvent {
            key: event.physical_key,
            state: event.state,
            repeat: event.repeat,
        });
    }

    /// Processes a [`RawKeyEvent`] (platform-independent, test-friendly).
    pub fn process_raw(&mut self, event: RawKeyEvent) {
        if event.repeat {
            return;
        }
        match event.state {
            ElementState::Pressed => {
                self.pressed.insert(event.key);
                self.just_pressed.insert(event.key);
            }
            ElementState::Released => {
                self.pressed.remove(&event.key);
                self.just_released.insert(event.key);
            }
        }
    }

    /// Returns `true` while the key is held down.
    #[must_use]
    pub fn is_pressed(&self, key: PhysicalKey) -> bool {
        self.pressed.contains(&key)
    }

    /// Returns `true` only during the frame the key transitioned to pressed.
    #[must_use]
    pub fn just_pressed(&self, key: PhysicalKey) -> bool {
        self.just_pressed.contains(&key)
    }

    /// Returns `true` only during the frame the key transitioned to released.
    #[must_use]
    pub fn just_released(&self, key: PhysicalKey) -> bool {
        self.just_released.contains(&key)
    }

    /// Clears `just_pressed` and `just_released` sets. Call at end of frame.
    pub fn clear_transients(&mut self) {
        self.just_pressed.clear();
        self.just_released.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode;

    /// Helper to create a [`RawKeyEvent`] for testing.
    fn raw(code: KeyCode, state: ElementState, repeat: bool) -> RawKeyEvent {
        RawKeyEvent {
            key: PhysicalKey::Code(code),
            state,
            repeat,
        }
    }

    #[test]
    fn test_initial_state_no_keys_pressed() {
        let kb = KeyboardState::new();
        let keys = [
            KeyCode::KeyW,
            KeyCode::KeyA,
            KeyCode::Space,
            KeyCode::Escape,
        ];
        for &k in &keys {
            let pk = PhysicalKey::Code(k);
            assert!(!kb.is_pressed(pk));
            assert!(!kb.just_pressed(pk));
            assert!(!kb.just_released(pk));
        }
    }

    #[test]
    fn test_press_event_sets_pressed() {
        let mut kb = KeyboardState::new();
        kb.process_raw(raw(KeyCode::KeyW, ElementState::Pressed, false));
        let pk = PhysicalKey::Code(KeyCode::KeyW);
        assert!(kb.is_pressed(pk));
        assert!(kb.just_pressed(pk));
    }

    #[test]
    fn test_release_clears_pressed() {
        let mut kb = KeyboardState::new();
        kb.process_raw(raw(KeyCode::KeyW, ElementState::Pressed, false));
        kb.process_raw(raw(KeyCode::KeyW, ElementState::Released, false));
        let pk = PhysicalKey::Code(KeyCode::KeyW);
        assert!(!kb.is_pressed(pk));
        assert!(kb.just_released(pk));
    }

    #[test]
    fn test_just_pressed_true_for_one_frame_only() {
        let mut kb = KeyboardState::new();
        kb.process_raw(raw(KeyCode::Space, ElementState::Pressed, false));
        let pk = PhysicalKey::Code(KeyCode::Space);
        assert!(kb.just_pressed(pk));
        kb.clear_transients();
        assert!(!kb.just_pressed(pk));
        assert!(kb.is_pressed(pk));
    }

    #[test]
    fn test_just_released_true_for_one_frame_only() {
        let mut kb = KeyboardState::new();
        kb.process_raw(raw(KeyCode::KeyW, ElementState::Pressed, false));
        kb.clear_transients();
        kb.process_raw(raw(KeyCode::KeyW, ElementState::Released, false));
        let pk = PhysicalKey::Code(KeyCode::KeyW);
        assert!(kb.just_released(pk));
        kb.clear_transients();
        assert!(!kb.just_released(pk));
        assert!(!kb.is_pressed(pk));
    }

    #[test]
    fn test_multiple_keys_tracked_independently() {
        let mut kb = KeyboardState::new();
        kb.process_raw(raw(KeyCode::KeyW, ElementState::Pressed, false));
        kb.process_raw(raw(KeyCode::KeyD, ElementState::Pressed, false));
        kb.process_raw(raw(KeyCode::KeyW, ElementState::Released, false));

        let w = PhysicalKey::Code(KeyCode::KeyW);
        let d = PhysicalKey::Code(KeyCode::KeyD);
        assert!(!kb.is_pressed(w));
        assert!(kb.is_pressed(d));
        assert!(kb.just_released(w));
        assert!(kb.just_pressed(d));
    }

    #[test]
    fn test_repeat_events_ignored() {
        let mut kb = KeyboardState::new();
        kb.process_raw(raw(KeyCode::KeyA, ElementState::Pressed, false));
        kb.process_raw(raw(KeyCode::KeyA, ElementState::Pressed, true));
        let pk = PhysicalKey::Code(KeyCode::KeyA);
        assert!(kb.just_pressed(pk));
        assert!(kb.is_pressed(pk));
    }
}
