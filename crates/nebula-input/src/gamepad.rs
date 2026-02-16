//! Gamepad input abstraction wrapping [`gilrs`].
//!
//! [`GamepadManager`] polls gilrs each frame, normalises axes through a
//! configurable deadzone, and tracks per-button press/release state.
//! Hot-plug is handled transparently: gamepads appear in
//! [`connected_gamepads`](GamepadManager::connected_gamepads) when plugged in
//! and disappear when unplugged.

use gilrs::{Axis, Button, EventType, GamepadId, Gilrs};
use glam::Vec2;
use std::collections::HashMap;

/// Unified button names that work across Xbox / PlayStation / generic pads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnifiedButton {
    /// A / Cross
    South,
    /// B / Circle
    East,
    /// Y / Triangle
    North,
    /// X / Square
    West,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
    LeftShoulder,
    RightShoulder,
    LeftTrigger,
    RightTrigger,
    LeftStick,
    RightStick,
    Start,
    Select,
}

impl UnifiedButton {
    fn from_gilrs(button: Button) -> Option<Self> {
        match button {
            Button::South => Some(Self::South),
            Button::East => Some(Self::East),
            Button::North => Some(Self::North),
            Button::West => Some(Self::West),
            Button::DPadUp => Some(Self::DPadUp),
            Button::DPadDown => Some(Self::DPadDown),
            Button::DPadLeft => Some(Self::DPadLeft),
            Button::DPadRight => Some(Self::DPadRight),
            Button::LeftTrigger => Some(Self::LeftShoulder),
            Button::RightTrigger => Some(Self::RightShoulder),
            Button::LeftTrigger2 => Some(Self::LeftTrigger),
            Button::RightTrigger2 => Some(Self::RightTrigger),
            Button::LeftThumb => Some(Self::LeftStick),
            Button::RightThumb => Some(Self::RightStick),
            Button::Start => Some(Self::Start),
            Button::Select => Some(Self::Select),
            _ => None,
        }
    }
}

/// Per-button frame state.
#[derive(Debug, Clone, Copy, Default)]
struct ButtonFrame {
    pressed: bool,
    just_pressed: bool,
    just_released: bool,
}

/// Axis values for a single gamepad.
#[derive(Debug, Clone, Copy, Default)]
pub struct GamepadAxes {
    /// Left stick. x: left(-1)..right(+1), y: down(-1)..up(+1).
    pub left_stick: Vec2,
    /// Right stick.
    pub right_stick: Vec2,
    /// Left trigger 0.0..1.0.
    pub left_trigger: f32,
    /// Right trigger 0.0..1.0.
    pub right_trigger: f32,
}

/// State snapshot for a single connected gamepad.
pub struct GamepadState {
    _id: GamepadId,
    name: String,
    connected: bool,
    axes: GamepadAxes,
    buttons: HashMap<UnifiedButton, ButtonFrame>,
}

impl GamepadState {
    fn new(id: GamepadId, name: String) -> Self {
        Self {
            _id: id,
            name,
            connected: true,
            axes: GamepadAxes::default(),
            buttons: HashMap::new(),
        }
    }

    /// Gamepad human-readable name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the gamepad is currently connected.
    pub fn connected(&self) -> bool {
        self.connected
    }

    /// Left analog stick after deadzone filtering.
    pub fn left_stick(&self) -> Vec2 {
        self.axes.left_stick
    }

    /// Right analog stick after deadzone filtering.
    pub fn right_stick(&self) -> Vec2 {
        self.axes.right_stick
    }

    /// Left trigger value `[0.0, 1.0]`.
    pub fn left_trigger(&self) -> f32 {
        self.axes.left_trigger
    }

    /// Right trigger value `[0.0, 1.0]`.
    pub fn right_trigger(&self) -> f32 {
        self.axes.right_trigger
    }

    /// Whether `button` is currently held.
    pub fn is_button_pressed(&self, button: UnifiedButton) -> bool {
        self.buttons.get(&button).is_some_and(|b| b.pressed)
    }

    /// Whether `button` was first pressed this frame.
    pub fn just_button_pressed(&self, button: UnifiedButton) -> bool {
        self.buttons.get(&button).is_some_and(|b| b.just_pressed)
    }

    /// Whether `button` was released this frame.
    pub fn just_button_released(&self, button: UnifiedButton) -> bool {
        self.buttons.get(&button).is_some_and(|b| b.just_released)
    }
}

/// Manages all connected gamepads via gilrs.
pub struct GamepadManager {
    gilrs: Gilrs,
    gamepads: HashMap<GamepadId, GamepadState>,
    /// Deadzone threshold for analog sticks (default 0.15).
    deadzone: f32,
}

impl GamepadManager {
    /// Create a new manager, initialising gilrs.
    ///
    /// # Panics
    /// Panics if gilrs cannot initialise (missing platform backend).
    pub fn new() -> Self {
        let gilrs = Gilrs::new().expect("Failed to initialise gilrs");
        let mut manager = Self {
            gilrs,
            gamepads: HashMap::new(),
            deadzone: 0.15,
        };
        // Register already-connected gamepads.
        let ids: Vec<_> = manager
            .gilrs
            .gamepads()
            .filter(|(_, g)| g.is_connected())
            .map(|(id, g)| (id, g.name().to_string()))
            .collect();
        for (id, name) in ids {
            manager.gamepads.insert(id, GamepadState::new(id, name));
        }
        manager
    }
}

impl Default for GamepadManager {
    fn default() -> Self {
        Self::new()
    }
}

impl GamepadManager {
    /// Set the analog stick deadzone. Values below this threshold are clamped
    /// to zero and the remaining range is rescaled to `[0.0, 1.0]`.
    pub fn set_deadzone(&mut self, value: f32) {
        self.deadzone = value.clamp(0.0, 0.99);
    }

    /// Current deadzone threshold.
    pub fn deadzone(&self) -> f32 {
        self.deadzone
    }

    /// Iterate over IDs of currently connected gamepads.
    pub fn connected_gamepads(&self) -> impl Iterator<Item = GamepadId> + '_ {
        self.gamepads
            .iter()
            .filter(|(_, s)| s.connected)
            .map(|(id, _)| *id)
    }

    /// Look up the state of a specific gamepad.
    pub fn gamepad(&self, id: GamepadId) -> Option<&GamepadState> {
        self.gamepads.get(&id)
    }

    /// Poll gilrs events and update all gamepad states. Call once per frame.
    pub fn update(&mut self) {
        // Clear per-frame flags.
        for state in self.gamepads.values_mut() {
            for bf in state.buttons.values_mut() {
                bf.just_pressed = false;
                bf.just_released = false;
            }
        }

        // Drain events.
        while let Some(event) = self.gilrs.next_event() {
            let id = event.id;
            match event.event {
                EventType::Connected => {
                    let name = self.gilrs.gamepad(id).name().to_string();
                    let entry = self
                        .gamepads
                        .entry(id)
                        .or_insert_with(|| GamepadState::new(id, name.clone()));
                    entry.connected = true;
                    entry.name = name;
                }
                EventType::Disconnected => {
                    if let Some(state) = self.gamepads.get_mut(&id) {
                        state.connected = false;
                    }
                }
                EventType::AxisChanged(axis, raw_value, _) => {
                    if let Some(state) = self.gamepads.get_mut(&id) {
                        let value = apply_deadzone(raw_value, self.deadzone);
                        match axis {
                            Axis::LeftStickX => state.axes.left_stick.x = value,
                            Axis::LeftStickY => state.axes.left_stick.y = value,
                            Axis::RightStickX => state.axes.right_stick.x = value,
                            Axis::RightStickY => state.axes.right_stick.y = value,
                            Axis::LeftZ => {
                                state.axes.left_trigger = value.max(0.0);
                            }
                            Axis::RightZ => {
                                state.axes.right_trigger = value.max(0.0);
                            }
                            _ => {}
                        }
                    }
                }
                EventType::ButtonPressed(button, _) => {
                    if let Some(unified) = UnifiedButton::from_gilrs(button)
                        && let Some(state) = self.gamepads.get_mut(&id)
                    {
                        let bf = state.buttons.entry(unified).or_default();
                        bf.pressed = true;
                        bf.just_pressed = true;
                    }
                }
                EventType::ButtonReleased(button, _) => {
                    if let Some(unified) = UnifiedButton::from_gilrs(button)
                        && let Some(state) = self.gamepads.get_mut(&id)
                    {
                        let bf = state.buttons.entry(unified).or_default();
                        bf.pressed = false;
                        bf.just_released = true;
                    }
                }
                _ => {}
            }
        }
    }
}

/// Apply deadzone filtering with rescaling.
///
/// If `|raw| < deadzone`, returns `0.0`.
/// Otherwise rescales from `[deadzone, 1.0]` to `[0.0, 1.0]`, preserving sign.
pub(crate) fn apply_deadzone(raw: f32, deadzone: f32) -> f32 {
    let abs = raw.abs();
    if abs < deadzone {
        return 0.0;
    }
    let scale = 1.0 / (1.0 - deadzone);
    let rescaled = (abs - deadzone) * scale;
    rescaled.min(1.0).copysign(raw)
}

// ── Mock-friendly test helpers ──────────────────────────────────────────────

/// A test-only gamepad manager that doesn't require gilrs hardware.
#[cfg(test)]
pub(crate) struct MockGamepadManager {
    pub gamepads: HashMap<u64, GamepadState>,
    pub deadzone: f32,
    next_id: u64,
}

#[cfg(test)]
impl MockGamepadManager {
    pub fn new() -> Self {
        Self {
            gamepads: HashMap::new(),
            deadzone: 0.15,
            next_id: 0,
        }
    }

    pub fn set_deadzone(&mut self, value: f32) {
        self.deadzone = value.clamp(0.0, 0.99);
    }

    /// Simulate a gamepad connection, returns an opaque id.
    pub fn connect(&mut self, name: &str) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.gamepads.insert(
            id,
            GamepadState {
                _id: unsafe { std::mem::transmute::<usize, GamepadId>(id as usize) },
                name: name.to_string(),
                connected: true,
                axes: GamepadAxes::default(),
                buttons: HashMap::new(),
            },
        );
        id
    }

    pub fn disconnect(&mut self, id: u64) {
        if let Some(s) = self.gamepads.get_mut(&id) {
            s.connected = false;
        }
    }

    pub fn set_axis(&mut self, id: u64, axis: &str, raw_value: f32) {
        if let Some(s) = self.gamepads.get_mut(&id) {
            let value = apply_deadzone(raw_value, self.deadzone);
            match axis {
                "left_stick_x" => s.axes.left_stick.x = value,
                "left_stick_y" => s.axes.left_stick.y = value,
                "right_stick_x" => s.axes.right_stick.x = value,
                "right_stick_y" => s.axes.right_stick.y = value,
                "left_trigger" => s.axes.left_trigger = value.max(0.0),
                "right_trigger" => s.axes.right_trigger = value.max(0.0),
                _ => {}
            }
        }
    }

    pub fn press_button(&mut self, id: u64, button: UnifiedButton) {
        if let Some(s) = self.gamepads.get_mut(&id) {
            let bf = s.buttons.entry(button).or_default();
            bf.pressed = true;
            bf.just_pressed = true;
        }
    }

    pub fn release_button(&mut self, id: u64, button: UnifiedButton) {
        if let Some(s) = self.gamepads.get_mut(&id) {
            let bf = s.buttons.entry(button).or_default();
            bf.pressed = false;
            bf.just_released = true;
        }
    }

    pub fn clear_frame(&mut self) {
        for s in self.gamepads.values_mut() {
            for bf in s.buttons.values_mut() {
                bf.just_pressed = false;
                bf.just_released = false;
            }
        }
    }

    pub fn connected_count(&self) -> usize {
        self.gamepads.values().filter(|s| s.connected).count()
    }

    pub fn gamepad(&self, id: u64) -> Option<&GamepadState> {
        self.gamepads.get(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gamepad_connection_detected() {
        let mut mgr = MockGamepadManager::new();
        let id = mgr.connect("Xbox Controller");
        assert_eq!(mgr.connected_count(), 1);
        assert!(mgr.gamepad(id).unwrap().connected());
    }

    #[test]
    fn test_axis_values_in_range() {
        let mut mgr = MockGamepadManager::new();
        mgr.set_deadzone(0.0); // disable deadzone for this test
        let id = mgr.connect("Pad");

        for &val in &[-1.0_f32, 0.0, 1.0] {
            mgr.set_axis(id, "left_stick_x", val);
            mgr.set_axis(id, "left_stick_y", val);
            let stick = mgr.gamepad(id).unwrap().left_stick();
            assert!((-1.0..=1.0).contains(&stick.x));
            assert!((-1.0..=1.0).contains(&stick.y));
        }
    }

    #[test]
    fn test_deadzone_filters_small_values() {
        let mut mgr = MockGamepadManager::new();
        mgr.set_deadzone(0.15);
        let id = mgr.connect("Pad");
        mgr.set_axis(id, "left_stick_x", 0.10);
        assert_eq!(mgr.gamepad(id).unwrap().left_stick().x, 0.0);
    }

    #[test]
    fn test_deadzone_rescales_above_threshold() {
        let mut mgr = MockGamepadManager::new();
        mgr.set_deadzone(0.15);
        let id = mgr.connect("Pad");
        mgr.set_axis(id, "left_stick_x", 0.575);
        let rescaled = mgr.gamepad(id).unwrap().left_stick().x;
        // (0.575 - 0.15) / (1.0 - 0.15) = 0.425 / 0.85 = 0.5
        assert!((rescaled - 0.5).abs() < 0.01, "got {rescaled}");
    }

    #[test]
    fn test_button_state_tracked() {
        let mut mgr = MockGamepadManager::new();
        let id = mgr.connect("Pad");

        mgr.press_button(id, UnifiedButton::South);
        let gs = mgr.gamepad(id).unwrap();
        assert!(gs.is_button_pressed(UnifiedButton::South));
        assert!(gs.just_button_pressed(UnifiedButton::South));

        mgr.clear_frame();
        mgr.release_button(id, UnifiedButton::South);
        let gs = mgr.gamepad(id).unwrap();
        assert!(!gs.is_button_pressed(UnifiedButton::South));
        assert!(gs.just_button_released(UnifiedButton::South));
    }

    #[test]
    fn test_disconnection_handled_gracefully() {
        let mut mgr = MockGamepadManager::new();
        let id = mgr.connect("Pad");
        mgr.disconnect(id);
        assert!(!mgr.gamepad(id).unwrap().connected());
        assert_eq!(mgr.connected_count(), 0);
    }

    #[test]
    fn test_multiple_gamepads_supported() {
        let mut mgr = MockGamepadManager::new();
        let _id1 = mgr.connect("Pad 1");
        let _id2 = mgr.connect("Pad 2");
        assert_eq!(mgr.connected_count(), 2);
    }

    #[test]
    fn test_custom_deadzone() {
        let mut mgr = MockGamepadManager::new();
        mgr.set_deadzone(0.25);
        let id = mgr.connect("Pad");

        mgr.set_axis(id, "left_stick_x", 0.20);
        assert_eq!(mgr.gamepad(id).unwrap().left_stick().x, 0.0);

        mgr.set_axis(id, "left_stick_x", 0.30);
        assert!(mgr.gamepad(id).unwrap().left_stick().x > 0.0);
    }
}
