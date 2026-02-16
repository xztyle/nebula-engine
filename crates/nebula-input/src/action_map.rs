//! Action mapping system: maps abstract game actions to physical input bindings.
//!
//! [`InputMap`] defines which physical inputs (keys, mouse buttons, gamepad axes)
//! trigger which [`Action`]s. [`ActionState`] is recomputed each frame by
//! [`ActionResolver`], which reads the current keyboard, mouse, and gamepad state.

use crate::gamepad::{GamepadState, UnifiedButton};
use crate::keyboard::KeyboardState;
use crate::mouse::MouseState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use winit::event::MouseButton;
use winit::keyboard::{KeyCode, PhysicalKey};

/// Serde helper module for [`KeyCode`] which doesn't implement serde natively.
mod keycode_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use winit::keyboard::KeyCode;

    /// Serialize a [`KeyCode`] as its debug string (e.g., `"KeyW"`).
    pub fn serialize<S: Serializer>(code: &KeyCode, s: S) -> Result<S::Ok, S::Error> {
        format!("{code:?}").serialize(s)
    }

    /// Deserialize a [`KeyCode`] from its debug string.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<KeyCode, D::Error> {
        let name = String::deserialize(d)?;
        string_to_keycode(&name)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown key: {name}")))
    }

    fn string_to_keycode(s: &str) -> Option<KeyCode> {
        // Match the Debug output of KeyCode variants
        Some(match s {
            "KeyA" => KeyCode::KeyA,
            "KeyB" => KeyCode::KeyB,
            "KeyC" => KeyCode::KeyC,
            "KeyD" => KeyCode::KeyD,
            "KeyE" => KeyCode::KeyE,
            "KeyF" => KeyCode::KeyF,
            "KeyG" => KeyCode::KeyG,
            "KeyH" => KeyCode::KeyH,
            "KeyI" => KeyCode::KeyI,
            "KeyJ" => KeyCode::KeyJ,
            "KeyK" => KeyCode::KeyK,
            "KeyL" => KeyCode::KeyL,
            "KeyM" => KeyCode::KeyM,
            "KeyN" => KeyCode::KeyN,
            "KeyO" => KeyCode::KeyO,
            "KeyP" => KeyCode::KeyP,
            "KeyQ" => KeyCode::KeyQ,
            "KeyR" => KeyCode::KeyR,
            "KeyS" => KeyCode::KeyS,
            "KeyT" => KeyCode::KeyT,
            "KeyU" => KeyCode::KeyU,
            "KeyV" => KeyCode::KeyV,
            "KeyW" => KeyCode::KeyW,
            "KeyX" => KeyCode::KeyX,
            "KeyY" => KeyCode::KeyY,
            "KeyZ" => KeyCode::KeyZ,
            "Digit0" => KeyCode::Digit0,
            "Digit1" => KeyCode::Digit1,
            "Digit2" => KeyCode::Digit2,
            "Digit3" => KeyCode::Digit3,
            "Digit4" => KeyCode::Digit4,
            "Digit5" => KeyCode::Digit5,
            "Digit6" => KeyCode::Digit6,
            "Digit7" => KeyCode::Digit7,
            "Digit8" => KeyCode::Digit8,
            "Digit9" => KeyCode::Digit9,
            "Space" => KeyCode::Space,
            "Enter" => KeyCode::Enter,
            "Escape" => KeyCode::Escape,
            "Tab" => KeyCode::Tab,
            "ShiftLeft" => KeyCode::ShiftLeft,
            "ShiftRight" => KeyCode::ShiftRight,
            "ControlLeft" => KeyCode::ControlLeft,
            "ControlRight" => KeyCode::ControlRight,
            "AltLeft" => KeyCode::AltLeft,
            "AltRight" => KeyCode::AltRight,
            "ArrowUp" => KeyCode::ArrowUp,
            "ArrowDown" => KeyCode::ArrowDown,
            "ArrowLeft" => KeyCode::ArrowLeft,
            "ArrowRight" => KeyCode::ArrowRight,
            _ => return None,
        })
    }
}

/// Semantic game actions that can be bound to physical inputs.
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum Action {
    /// Move the player forward.
    MoveForward,
    /// Move the player backward.
    MoveBack,
    /// Strafe left.
    MoveLeft,
    /// Strafe right.
    MoveRight,
    /// Jump.
    Jump,
    /// Crouch or duck.
    Crouch,
    /// Sprint / run faster.
    Sprint,
    /// Primary action (e.g., attack, place block).
    PrimaryAction,
    /// Secondary action (e.g., aim, use item).
    SecondaryAction,
    /// Interact with objects.
    Interact,
    /// Open inventory screen.
    OpenInventory,
    /// Pause the game.
    Pause,
}

/// Which mouse axis to read for an analog binding.
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum MouseAxisBinding {
    /// Horizontal mouse delta.
    X,
    /// Vertical mouse delta.
    Y,
    /// Scroll wheel.
    Scroll,
}

/// Which gamepad axis to read for an analog binding.
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum GamepadAxisBinding {
    /// Left stick horizontal.
    LeftStickX,
    /// Left stick vertical.
    LeftStickY,
    /// Right stick horizontal.
    RightStickX,
    /// Right stick vertical.
    RightStickY,
    /// Left trigger (0..1).
    LeftTrigger,
    /// Right trigger (0..1).
    RightTrigger,
}

/// A physical input source that can be bound to an action.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum InputBinding {
    /// A keyboard key (physical scan code).
    Key(#[serde(with = "keycode_serde")] KeyCode),
    /// A mouse button.
    MouseButton(MouseButtonBinding),
    /// A mouse axis (analog).
    MouseAxis(MouseAxisBinding),
    /// A gamepad button (digital).
    GamepadButton(UnifiedButton),
    /// A gamepad axis (analog).
    GamepadAxis(GamepadAxisBinding),
}

/// Wrapper for [`winit::event::MouseButton`] that supports serde.
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum MouseButtonBinding {
    /// Left mouse button.
    Left,
    /// Right mouse button.
    Right,
    /// Middle mouse button.
    Middle,
}

impl MouseButtonBinding {
    /// Convert to the winit [`MouseButton`] type.
    #[must_use]
    pub fn to_winit(self) -> MouseButton {
        match self {
            Self::Left => MouseButton::Left,
            Self::Right => MouseButton::Right,
            Self::Middle => MouseButton::Middle,
        }
    }
}

/// Maps [`Action`]s to lists of [`InputBinding`]s.
///
/// Multiple bindings per action are supported (OR logic for digital, sum+clamp
/// for analog). Serializable to RON for user-editable config files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputMap {
    /// The binding table.
    pub bindings: HashMap<Action, Vec<InputBinding>>,
}

impl Default for InputMap {
    fn default() -> Self {
        Self::default_fps()
    }
}

impl InputMap {
    /// Create an empty input map with no bindings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    /// Standard FPS-style default bindings (WASD + mouse + gamepad).
    #[must_use]
    pub fn default_fps() -> Self {
        let mut bindings: HashMap<Action, Vec<InputBinding>> = HashMap::new();

        bindings.insert(
            Action::MoveForward,
            vec![
                InputBinding::Key(KeyCode::KeyW),
                InputBinding::GamepadAxis(GamepadAxisBinding::LeftStickY),
            ],
        );
        bindings.insert(
            Action::MoveBack,
            vec![
                InputBinding::Key(KeyCode::KeyS),
                InputBinding::GamepadAxis(GamepadAxisBinding::LeftStickY),
            ],
        );
        bindings.insert(
            Action::MoveLeft,
            vec![
                InputBinding::Key(KeyCode::KeyA),
                InputBinding::GamepadAxis(GamepadAxisBinding::LeftStickX),
            ],
        );
        bindings.insert(
            Action::MoveRight,
            vec![
                InputBinding::Key(KeyCode::KeyD),
                InputBinding::GamepadAxis(GamepadAxisBinding::LeftStickX),
            ],
        );
        bindings.insert(
            Action::Jump,
            vec![
                InputBinding::Key(KeyCode::Space),
                InputBinding::GamepadButton(UnifiedButton::South),
            ],
        );
        bindings.insert(
            Action::Crouch,
            vec![
                InputBinding::Key(KeyCode::ControlLeft),
                InputBinding::GamepadButton(UnifiedButton::East),
            ],
        );
        bindings.insert(
            Action::Sprint,
            vec![
                InputBinding::Key(KeyCode::ShiftLeft),
                InputBinding::GamepadButton(UnifiedButton::LeftStick),
            ],
        );
        bindings.insert(
            Action::PrimaryAction,
            vec![
                InputBinding::MouseButton(MouseButtonBinding::Left),
                InputBinding::GamepadButton(UnifiedButton::RightShoulder),
            ],
        );
        bindings.insert(
            Action::SecondaryAction,
            vec![
                InputBinding::MouseButton(MouseButtonBinding::Right),
                InputBinding::GamepadButton(UnifiedButton::LeftShoulder),
            ],
        );
        bindings.insert(
            Action::Interact,
            vec![
                InputBinding::Key(KeyCode::KeyE),
                InputBinding::GamepadButton(UnifiedButton::West),
            ],
        );
        bindings.insert(
            Action::OpenInventory,
            vec![
                InputBinding::Key(KeyCode::Tab),
                InputBinding::GamepadButton(UnifiedButton::Select),
            ],
        );
        bindings.insert(
            Action::Pause,
            vec![
                InputBinding::Key(KeyCode::Escape),
                InputBinding::GamepadButton(UnifiedButton::Start),
            ],
        );

        Self { bindings }
    }

    /// Set the bindings for an action, replacing any existing ones.
    pub fn set_bindings(&mut self, action: Action, bindings: Vec<InputBinding>) {
        self.bindings.insert(action, bindings);
    }

    /// Get the bindings for an action.
    #[must_use]
    pub fn get_bindings(&self, action: &Action) -> &[InputBinding] {
        self.bindings.get(action).map_or(&[], |v| v.as_slice())
    }

    /// Serialize to RON string.
    ///
    /// # Errors
    /// Returns an error if serialization fails.
    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
    }

    /// Deserialize from RON string.
    ///
    /// # Errors
    /// Returns an error if the RON string is malformed.
    pub fn from_ron(s: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(s)
    }
}

/// Threshold below which an action is considered inactive.
const ACTIVATION_THRESHOLD: f32 = 0.001;

/// Per-frame action state computed by [`ActionResolver`].
#[derive(Debug, Clone)]
pub struct ActionState {
    /// Current frame values.
    values: HashMap<Action, f32>,
    /// Previous frame values (for edge detection).
    prev_values: HashMap<Action, f32>,
}

impl Default for ActionState {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionState {
    /// Create a new empty action state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
            prev_values: HashMap::new(),
        }
    }

    /// Whether an action's value is above the activation threshold.
    #[must_use]
    pub fn is_action_active(&self, action: Action) -> bool {
        self.action_value(action).abs() > ACTIVATION_THRESHOLD
    }

    /// The analog value of an action, clamped to `[-1.0, 1.0]`.
    #[must_use]
    pub fn action_value(&self, action: Action) -> f32 {
        self.values.get(&action).copied().unwrap_or(0.0)
    }

    /// True only on the frame the action transitioned from inactive to active.
    #[must_use]
    pub fn action_just_activated(&self, action: Action) -> bool {
        let cur = self.action_value(action).abs() > ACTIVATION_THRESHOLD;
        let prev =
            self.prev_values.get(&action).copied().unwrap_or(0.0).abs() > ACTIVATION_THRESHOLD;
        cur && !prev
    }

    /// True only on the frame the action transitioned from active to inactive.
    #[must_use]
    pub fn action_just_deactivated(&self, action: Action) -> bool {
        let cur = self.action_value(action).abs() > ACTIVATION_THRESHOLD;
        let prev =
            self.prev_values.get(&action).copied().unwrap_or(0.0).abs() > ACTIVATION_THRESHOLD;
        !cur && prev
    }
}

/// Reads input state resources and populates [`ActionState`] each frame.
pub struct ActionResolver;

impl ActionResolver {
    /// Resolve all actions from the current input state.
    ///
    /// Call once per frame after input state has been updated.
    pub fn resolve(
        input_map: &InputMap,
        keyboard: &KeyboardState,
        mouse: &MouseState,
        gamepad: Option<&GamepadState>,
        state: &mut ActionState,
    ) {
        // Shift current values to previous.
        state.prev_values.clone_from(&state.values);
        state.values.clear();

        for (action, bindings) in &input_map.bindings {
            let mut value = 0.0_f32;

            for binding in bindings {
                let v = Self::read_binding(binding, keyboard, mouse, gamepad);
                // Sum for analog, which also covers OR for digital (max via clamp).
                value += v;
            }

            // Clamp to [-1, 1].
            value = value.clamp(-1.0, 1.0);
            state.values.insert(*action, value);
        }
    }

    /// Read the current value of a single binding.
    fn read_binding(
        binding: &InputBinding,
        keyboard: &KeyboardState,
        mouse: &MouseState,
        gamepad: Option<&GamepadState>,
    ) -> f32 {
        match binding {
            InputBinding::Key(code) => {
                if keyboard.is_pressed(PhysicalKey::Code(*code)) {
                    1.0
                } else {
                    0.0
                }
            }
            InputBinding::MouseButton(btn) => {
                if mouse.is_button_pressed(btn.to_winit()) {
                    1.0
                } else {
                    0.0
                }
            }
            InputBinding::MouseAxis(axis) => {
                let d = mouse.delta();
                match axis {
                    MouseAxisBinding::X => d.x,
                    MouseAxisBinding::Y => d.y,
                    MouseAxisBinding::Scroll => mouse.scroll(),
                }
            }
            InputBinding::GamepadButton(btn) => {
                if let Some(gp) = gamepad
                    && gp.is_button_pressed(*btn)
                {
                    1.0
                } else {
                    0.0
                }
            }
            InputBinding::GamepadAxis(axis) => {
                if let Some(gp) = gamepad {
                    match axis {
                        GamepadAxisBinding::LeftStickX => gp.left_stick().x,
                        GamepadAxisBinding::LeftStickY => gp.left_stick().y,
                        GamepadAxisBinding::RightStickX => gp.right_stick().x,
                        GamepadAxisBinding::RightStickY => gp.right_stick().y,
                        GamepadAxisBinding::LeftTrigger => gp.left_trigger(),
                        GamepadAxisBinding::RightTrigger => gp.right_trigger(),
                    }
                } else {
                    0.0
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::event::ElementState;

    /// Helper: press a key on a keyboard state.
    fn press_key(kb: &mut KeyboardState, code: KeyCode) {
        kb.process_raw(crate::keyboard::RawKeyEvent {
            key: PhysicalKey::Code(code),
            state: ElementState::Pressed,
            repeat: false,
        });
    }

    /// Helper: release a key on a keyboard state.
    fn release_key(kb: &mut KeyboardState, code: KeyCode) {
        kb.process_raw(crate::keyboard::RawKeyEvent {
            key: PhysicalKey::Code(code),
            state: ElementState::Released,
            repeat: false,
        });
    }

    #[test]
    fn test_action_bound_to_key_activates_on_press() {
        let mut map = InputMap::new();
        map.set_bindings(Action::MoveForward, vec![InputBinding::Key(KeyCode::KeyW)]);

        let mut kb = KeyboardState::new();
        press_key(&mut kb, KeyCode::KeyW);

        let mouse = MouseState::new();
        let mut state = ActionState::new();
        ActionResolver::resolve(&map, &kb, &mouse, None, &mut state);

        assert!(state.is_action_active(Action::MoveForward));
        assert!((state.action_value(Action::MoveForward) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_action_bound_to_gamepad_axis_returns_analog() {
        let mut map = InputMap::new();
        map.set_bindings(
            Action::MoveForward,
            vec![InputBinding::GamepadAxis(GamepadAxisBinding::LeftStickY)],
        );

        let kb = KeyboardState::new();
        let mouse = MouseState::new();

        // Build a mock gamepad state with left_stick.y = 0.75
        // We need to use the mock manager from gamepad module
        use crate::gamepad::MockGamepadManager;
        let mut mgr = MockGamepadManager::new();
        mgr.set_deadzone(0.0);
        let id = mgr.connect("TestPad");
        mgr.set_axis(id, "left_stick_y", 0.75);
        let gp = mgr.gamepad(id).unwrap();

        let mut state = ActionState::new();
        ActionResolver::resolve(&map, &kb, &mouse, Some(gp), &mut state);

        assert!(
            (state.action_value(Action::MoveForward) - 0.75).abs() < 0.01,
            "got {}",
            state.action_value(Action::MoveForward)
        );
    }

    #[test]
    fn test_unbound_action_returns_false_and_zero() {
        let map = InputMap::new();
        let kb = KeyboardState::new();
        let mouse = MouseState::new();
        let mut state = ActionState::new();
        ActionResolver::resolve(&map, &kb, &mouse, None, &mut state);

        assert!(!state.is_action_active(Action::OpenInventory));
        assert!((state.action_value(Action::OpenInventory)).abs() < f32::EPSILON);
    }

    #[test]
    fn test_multiple_bindings_or_logic() {
        let mut map = InputMap::new();
        map.set_bindings(
            Action::Jump,
            vec![
                InputBinding::Key(KeyCode::Space),
                InputBinding::GamepadButton(UnifiedButton::South),
            ],
        );

        let mut kb = KeyboardState::new();
        press_key(&mut kb, KeyCode::Space);

        let mouse = MouseState::new();
        let mut state = ActionState::new();
        ActionResolver::resolve(&map, &kb, &mouse, None, &mut state);

        assert!(state.is_action_active(Action::Jump));
    }

    #[test]
    fn test_multiple_bindings_both_active() {
        let mut map = InputMap::new();
        map.set_bindings(
            Action::Jump,
            vec![
                InputBinding::Key(KeyCode::Space),
                InputBinding::GamepadButton(UnifiedButton::South),
            ],
        );

        let mut kb = KeyboardState::new();
        press_key(&mut kb, KeyCode::Space);

        let mouse = MouseState::new();

        use crate::gamepad::MockGamepadManager;
        let mut mgr = MockGamepadManager::new();
        let id = mgr.connect("TestPad");
        mgr.press_button(id, UnifiedButton::South);
        let gp = mgr.gamepad(id).unwrap();

        let mut state = ActionState::new();
        ActionResolver::resolve(&map, &kb, &mouse, Some(gp), &mut state);

        assert!(
            (state.action_value(Action::Jump) - 1.0).abs() < f32::EPSILON,
            "should be clamped to 1.0, got {}",
            state.action_value(Action::Jump)
        );
    }

    #[test]
    fn test_action_map_modified_at_runtime() {
        let mut map = InputMap::new();
        map.set_bindings(Action::Jump, vec![InputBinding::Key(KeyCode::Space)]);

        let mut kb = KeyboardState::new();
        let mouse = MouseState::new();
        let mut state = ActionState::new();

        // Rebind Jump to KeyJ
        map.set_bindings(Action::Jump, vec![InputBinding::Key(KeyCode::KeyJ)]);

        press_key(&mut kb, KeyCode::KeyJ);
        ActionResolver::resolve(&map, &kb, &mouse, None, &mut state);
        assert!(state.is_action_active(Action::Jump));

        // Space should no longer activate Jump
        release_key(&mut kb, KeyCode::KeyJ);
        press_key(&mut kb, KeyCode::Space);
        ActionResolver::resolve(&map, &kb, &mouse, None, &mut state);
        assert!(!state.is_action_active(Action::Jump));
    }

    #[test]
    fn test_action_just_activated_edge() {
        let mut map = InputMap::new();
        map.set_bindings(Action::Jump, vec![InputBinding::Key(KeyCode::Space)]);

        let mut kb = KeyboardState::new();
        let mouse = MouseState::new();
        let mut state = ActionState::new();

        // Frame 1: press Space
        press_key(&mut kb, KeyCode::Space);
        ActionResolver::resolve(&map, &kb, &mouse, None, &mut state);
        assert!(
            state.action_just_activated(Action::Jump),
            "should be just activated on frame 1"
        );

        // Frame 2: still held
        ActionResolver::resolve(&map, &kb, &mouse, None, &mut state);
        assert!(
            !state.action_just_activated(Action::Jump),
            "should NOT be just activated on frame 2"
        );
        assert!(state.is_action_active(Action::Jump));
    }

    #[test]
    fn test_analog_sum_clamped() {
        let mut map = InputMap::new();
        map.set_bindings(
            Action::MoveForward,
            vec![
                InputBinding::Key(KeyCode::KeyW),
                InputBinding::GamepadAxis(GamepadAxisBinding::LeftStickY),
            ],
        );

        let mut kb = KeyboardState::new();
        press_key(&mut kb, KeyCode::KeyW); // contributes 1.0
        let mouse = MouseState::new();

        use crate::gamepad::MockGamepadManager;
        let mut mgr = MockGamepadManager::new();
        mgr.set_deadzone(0.0);
        let id = mgr.connect("TestPad");
        mgr.set_axis(id, "left_stick_y", 0.8); // contributes 0.8
        let gp = mgr.gamepad(id).unwrap();

        let mut state = ActionState::new();
        ActionResolver::resolve(&map, &kb, &mouse, Some(gp), &mut state);

        assert!(
            (state.action_value(Action::MoveForward) - 1.0).abs() < f32::EPSILON,
            "should be clamped to 1.0, got {}",
            state.action_value(Action::MoveForward)
        );
    }
}
