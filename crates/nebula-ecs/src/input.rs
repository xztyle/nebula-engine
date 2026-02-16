//! Aggregated input state resource for the current frame.

use bevy_ecs::prelude::*;
use std::collections::HashSet;

/// Aggregated input state for the current frame. Written by PreUpdate,
/// read by FixedUpdate and Update.
///
/// Provides action-based input queries rather than raw key codes.
/// The mapping from physical keys to actions is configured externally
/// (in nebula-input). This resource exposes the processed result.
#[derive(Resource, Clone, Debug, Default)]
pub struct InputState {
    /// Actions that are currently held down.
    pub active_actions: HashSet<String>,
    /// Actions that were first pressed this frame.
    pub just_pressed: HashSet<String>,
    /// Actions that were released this frame.
    pub just_released: HashSet<String>,
    /// Mouse movement delta in pixels since last frame.
    pub mouse_delta: (f32, f32),
    /// Mouse scroll delta (horizontal, vertical).
    pub scroll_delta: (f32, f32),
    /// Current cursor position in window coordinates, if available.
    pub cursor_position: Option<(f32, f32)>,
}

impl InputState {
    /// Returns true if the named action is currently held down.
    pub fn is_active(&self, action: &str) -> bool {
        self.active_actions.contains(action)
    }

    /// Returns true if the named action was first pressed this frame.
    pub fn just_pressed(&self, action: &str) -> bool {
        self.just_pressed.contains(action)
    }

    /// Returns true if the named action was released this frame.
    pub fn just_released(&self, action: &str) -> bool {
        self.just_released.contains(action)
    }

    /// Clear per-frame transient state. Called at the start of PreUpdate
    /// before processing new input events.
    pub fn clear_transients(&mut self) {
        self.just_pressed.clear();
        self.just_released.clear();
        self.mouse_delta = (0.0, 0.0);
        self.scroll_delta = (0.0, 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_state_action_queries() {
        let mut input = InputState::default();
        input.active_actions.insert("jump".to_string());
        input.just_pressed.insert("fire".to_string());
        input.just_released.insert("crouch".to_string());

        assert!(input.is_active("jump"));
        assert!(!input.is_active("fire"));
        assert!(input.just_pressed("fire"));
        assert!(!input.just_pressed("jump"));
        assert!(input.just_released("crouch"));
    }

    #[test]
    fn test_input_state_clear_transients() {
        let mut input = InputState::default();
        input.just_pressed.insert("fire".to_string());
        input.just_released.insert("crouch".to_string());
        input.mouse_delta = (10.0, 20.0);
        input.scroll_delta = (0.0, 3.0);

        input.clear_transients();

        assert!(input.just_pressed.is_empty());
        assert!(input.just_released.is_empty());
        assert_eq!(input.mouse_delta, (0.0, 0.0));
        assert_eq!(input.scroll_delta, (0.0, 0.0));
    }
}
