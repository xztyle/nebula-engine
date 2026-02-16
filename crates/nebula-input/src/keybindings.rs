//! Configurable keybinding persistence, conflict detection, and rebind flow.
//!
//! Provides [`Modifiers`] bitflags, [`Conflict`] detection, and RON-based
//! save/load for [`InputMap`] with fallback to defaults on error.

use crate::action_map::{Action, InputBinding, InputMap};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tracing::warn;

// ── Modifiers ───────────────────────────────────────────────────────

/// Modifier key bitflags. Combines via bitwise OR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Modifiers(pub u8);

impl Modifiers {
    /// No modifiers.
    pub const NONE: Self = Self(0);
    /// Shift key.
    pub const SHIFT: Self = Self(1 << 0);
    /// Control key.
    pub const CTRL: Self = Self(1 << 1);
    /// Alt key.
    pub const ALT: Self = Self(1 << 2);
    /// Super/Meta/Win key.
    pub const SUPER: Self = Self(1 << 3);

    /// Returns true if `self` contains all bits in `other`.
    #[must_use]
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Returns true if no modifier bits are set.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Modifiers {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl std::ops::BitAnd for Modifiers {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

// ── Conflict ────────────────────────────────────────────────────────

/// A binding conflict: the same [`InputBinding`] is used by multiple actions.
#[derive(Debug, Clone)]
pub struct Conflict {
    /// The duplicated binding.
    pub binding: InputBinding,
    /// Actions that share this binding.
    pub actions: Vec<Action>,
}

impl InputMap {
    /// Detect all binding conflicts (same binding in multiple actions, or
    /// duplicates within a single action).
    #[must_use]
    pub fn detect_conflicts(&self) -> Vec<Conflict> {
        let mut seen: HashMap<InputBinding, Vec<Action>> = HashMap::new();

        for (action, bindings) in &self.bindings {
            for binding in bindings {
                seen.entry(*binding).or_default().push(*action);
            }
        }

        seen.into_iter()
            .filter(|(_, actions)| actions.len() > 1)
            .map(|(binding, actions)| Conflict { binding, actions })
            .collect()
    }

    /// Save the input map to a RON file at `path`.
    ///
    /// # Errors
    /// Returns an error if serialization or file writing fails.
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let ron_str = self.to_ron()?;
        std::fs::write(path, ron_str)?;
        Ok(())
    }

    /// Load an input map from a RON file at `path`.
    ///
    /// Falls back to [`InputMap::default`] if the file is missing or malformed,
    /// logging a warning in either case.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => match Self::from_ron(&contents) {
                Ok(map) => map,
                Err(e) => {
                    warn!(
                        "Malformed keybinding file {}: {e}; using defaults",
                        path.display()
                    );
                    Self::default()
                }
            },
            Err(e) => {
                warn!(
                    "Could not read keybinding file {}: {e}; using defaults",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Returns the platform config path for `input.ron`.
    #[must_use]
    pub fn default_config_path() -> Option<std::path::PathBuf> {
        dirs::config_dir().map(|d| d.join("nebula").join("input.ron"))
    }
}

// ── Rebind flow ─────────────────────────────────────────────────────

/// State machine for the rebind-listen flow.
#[derive(Debug, Clone, Default)]
pub enum RebindState {
    /// Not rebinding.
    #[default]
    Idle,
    /// Listening for the next input to bind to `action`.
    Listening { action: Action },
}

impl RebindState {
    /// Begin listening for a new binding for `action`.
    pub fn start_rebind(&mut self, action: Action) {
        *self = Self::Listening { action };
    }

    /// Returns the action being rebound, if in listening mode.
    #[must_use]
    pub fn listening_action(&self) -> Option<Action> {
        match self {
            Self::Listening { action } => Some(*action),
            Self::Idle => None,
        }
    }

    /// Capture a binding. Returns `Some((action, conflicts))` if a binding was
    /// captured, allowing the caller to decide whether to accept or reject.
    /// Resets to `Idle` regardless.
    pub fn capture(
        &mut self,
        binding: InputBinding,
        input_map: &mut InputMap,
    ) -> Option<(Action, Vec<Conflict>)> {
        let action = self.listening_action()?;

        // Apply the binding.
        input_map.set_bindings(action, vec![binding]);

        // Detect conflicts after applying.
        let conflicts = input_map.detect_conflicts();

        *self = Self::Idle;
        Some((action, conflicts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_map::{ActionResolver, ActionState};
    use crate::keyboard::KeyboardState;
    use crate::mouse::MouseState;
    use winit::event::ElementState as WinitElementState;
    use winit::keyboard::{KeyCode, PhysicalKey};

    fn press_key(kb: &mut KeyboardState, code: KeyCode) {
        kb.process_raw(crate::keyboard::RawKeyEvent {
            key: PhysicalKey::Code(code),
            state: WinitElementState::Pressed,
            repeat: false,
        });
    }

    #[test]
    fn test_default_bindings_serialize_to_ron() {
        let original = InputMap::default();
        let ron_str = original.to_ron().expect("serialize");
        let restored = InputMap::from_ron(&ron_str).expect("deserialize");
        // Every action in original should be present with same binding count.
        for (action, bindings) in &original.bindings {
            let restored_bindings = restored.get_bindings(action);
            assert_eq!(
                bindings.len(),
                restored_bindings.len(),
                "action {action:?} binding count mismatch"
            );
        }
    }

    #[test]
    fn test_custom_bindings_deserialize_correctly() {
        let mut map = InputMap::new();
        map.set_bindings(Action::Jump, vec![InputBinding::Key(KeyCode::KeyJ)]);
        let ron_str = map.to_ron().expect("serialize");
        let restored = InputMap::from_ron(&ron_str).expect("deserialize");
        let bindings = restored.get_bindings(&Action::Jump);
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0], InputBinding::Key(KeyCode::KeyJ));
    }

    #[test]
    fn test_conflict_detection_flags_duplicates() {
        let mut map = InputMap::new();
        map.set_bindings(Action::Jump, vec![InputBinding::Key(KeyCode::Space)]);
        map.set_bindings(Action::Sprint, vec![InputBinding::Key(KeyCode::Space)]);
        let conflicts = map.detect_conflicts();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].binding, InputBinding::Key(KeyCode::Space));
        assert!(conflicts[0].actions.contains(&Action::Jump));
        assert!(conflicts[0].actions.contains(&Action::Sprint));
    }

    #[test]
    fn test_no_conflicts_on_clean_map() {
        let _default_map = InputMap::default();
        // Use a known-clean map (default FPS map shares gamepad axes across actions).
        let mut clean_map = InputMap::new();
        clean_map.set_bindings(Action::Jump, vec![InputBinding::Key(KeyCode::Space)]);
        clean_map.set_bindings(Action::Sprint, vec![InputBinding::Key(KeyCode::ShiftLeft)]);
        clean_map.set_bindings(Action::Interact, vec![InputBinding::Key(KeyCode::KeyE)]);
        let conflicts = clean_map.detect_conflicts();
        assert!(conflicts.is_empty());
    }

    #[test]
    fn test_modifier_combinations_work() {
        let mut map = InputMap::new();
        map.set_bindings(
            Action::OpenInventory,
            vec![InputBinding::KeyWithModifiers {
                key: KeyCode::KeyI,
                modifiers: Modifiers::CTRL,
            }],
        );

        let mouse = MouseState::new();
        let mut state = ActionState::new();

        // Press I alone — should NOT activate.
        let mut kb = KeyboardState::new();
        press_key(&mut kb, KeyCode::KeyI);
        ActionResolver::resolve(&map, &kb, &mouse, None, &mut state);
        assert!(!state.is_action_active(Action::OpenInventory));

        // Press Ctrl+I — should activate.
        let mut kb2 = KeyboardState::new();
        press_key(&mut kb2, KeyCode::ControlLeft);
        press_key(&mut kb2, KeyCode::KeyI);
        ActionResolver::resolve(&map, &kb2, &mouse, None, &mut state);
        assert!(state.is_action_active(Action::OpenInventory));
    }

    #[test]
    fn test_modifier_subset_does_not_match() {
        let mut map = InputMap::new();
        map.set_bindings(
            Action::OpenInventory,
            vec![InputBinding::KeyWithModifiers {
                key: KeyCode::KeyS,
                modifiers: Modifiers::CTRL | Modifiers::SHIFT,
            }],
        );

        let mouse = MouseState::new();
        let mut state = ActionState::new();

        // Press only Ctrl+S (missing Shift) — should NOT activate.
        let mut kb = KeyboardState::new();
        press_key(&mut kb, KeyCode::ControlLeft);
        press_key(&mut kb, KeyCode::KeyS);
        ActionResolver::resolve(&map, &kb, &mouse, None, &mut state);
        assert!(!state.is_action_active(Action::OpenInventory));
    }

    #[test]
    fn test_rebinding_persists_across_save_load() {
        let dir = std::env::temp_dir().join("nebula_keybind_test");
        let path = dir.join("input.ron");

        let mut map = InputMap::default();
        map.set_bindings(Action::Jump, vec![InputBinding::Key(KeyCode::KeyK)]);
        map.save(&path).expect("save");

        let loaded = InputMap::load(&path);
        let bindings = loaded.get_bindings(&Action::Jump);
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0], InputBinding::Key(KeyCode::KeyK));

        // Cleanup.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_malformed_ron_falls_back_to_defaults() {
        let dir = std::env::temp_dir().join("nebula_keybind_malformed");
        let path = dir.join("input.ron");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "not valid ron {{{").unwrap();

        let loaded = InputMap::load(&path);
        // Should be the default map, not panic.
        assert!(!loaded.bindings.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_missing_file_falls_back_to_defaults() {
        let path = std::path::PathBuf::from("/tmp/nebula_nonexistent_12345/input.ron");
        let loaded = InputMap::load(&path);
        assert!(!loaded.bindings.is_empty());
    }
}
