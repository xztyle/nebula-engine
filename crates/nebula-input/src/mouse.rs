//! Frame-coherent mouse state tracker.
//!
//! [`MouseState`] accumulates winit mouse events during a frame and exposes a
//! clean query API for position, delta, button states, scroll wheel, cursor
//! capture, and cursor-in-window status.

use glam::Vec2;
use winit::event::{ElementState, MouseButton, MouseScrollDelta};

/// Per-button press/release tracking for a single frame.
#[derive(Debug, Clone, Copy, Default)]
struct ButtonFrame {
    pressed: bool,
    just_pressed: bool,
    just_released: bool,
}

/// Maps a [`MouseButton`] to an index 0..4.
fn button_index(button: MouseButton) -> usize {
    match button {
        MouseButton::Left => 0,
        MouseButton::Right => 1,
        MouseButton::Middle => 2,
        MouseButton::Back => 3,
        MouseButton::Forward => 4,
        MouseButton::Other(_) => 4,
    }
}

/// Frame-coherent mouse state.
///
/// # Usage
///
/// 1. Forward winit events via the `on_*` methods during event collection.
/// 2. Query state with the public accessors.
/// 3. Call [`clear_transients`](Self::clear_transients) at end of frame.
#[derive(Debug, Clone)]
pub struct MouseState {
    position: Vec2,
    prev_position: Vec2,
    delta: Vec2,
    buttons: [ButtonFrame; 5],
    scroll: f32,
    captured: bool,
    cursor_in_window: bool,
}

impl Default for MouseState {
    fn default() -> Self {
        Self::new()
    }
}

impl MouseState {
    /// Creates a new `MouseState` with all fields zeroed/false.
    #[must_use]
    pub fn new() -> Self {
        Self {
            position: Vec2::ZERO,
            prev_position: Vec2::ZERO,
            delta: Vec2::ZERO,
            buttons: [ButtonFrame::default(); 5],
            scroll: 0.0,
            captured: false,
            cursor_in_window: false,
        }
    }

    // ── Event handlers ──────────────────────────────────────────────

    /// Process a `CursorMoved` event.
    pub fn on_cursor_moved(&mut self, x: f64, y: f64) {
        let new_pos = Vec2::new(x as f32, y as f32);
        if !self.captured {
            self.delta += new_pos - self.position;
        }
        self.position = new_pos;
    }

    /// Process a `DeviceEvent::MouseMotion` raw delta (used when captured).
    pub fn on_raw_motion(&mut self, dx: f64, dy: f64) {
        if self.captured {
            self.delta += Vec2::new(dx as f32, dy as f32);
        }
    }

    /// Process a `MouseInput` event.
    pub fn on_button(&mut self, button: MouseButton, state: ElementState) {
        let idx = button_index(button);
        match state {
            ElementState::Pressed => {
                self.buttons[idx].pressed = true;
                self.buttons[idx].just_pressed = true;
            }
            ElementState::Released => {
                self.buttons[idx].pressed = false;
                self.buttons[idx].just_released = true;
            }
        }
    }

    /// Process a `MouseWheel` event.
    pub fn on_scroll(&mut self, delta: MouseScrollDelta) {
        match delta {
            MouseScrollDelta::LineDelta(_x, y) => {
                self.scroll += y;
            }
            MouseScrollDelta::PixelDelta(pos) => {
                // Normalize pixel delta: ~40 pixels ≈ 1 line
                self.scroll += (pos.y / 40.0) as f32;
            }
        }
    }

    /// Process a `CursorEntered` event.
    pub fn on_cursor_entered(&mut self) {
        self.cursor_in_window = true;
    }

    /// Process a `CursorLeft` event.
    pub fn on_cursor_left(&mut self) {
        self.cursor_in_window = false;
    }

    /// Set cursor capture state. Pass the window to apply grab/visibility.
    ///
    /// When captured, the cursor is hidden and confined (or locked) to the
    /// window, and raw `DeviceEvent::MouseMotion` deltas are used instead of
    /// `CursorMoved` position differences.
    pub fn set_captured(&mut self, window: &winit::window::Window, captured: bool) {
        use winit::window::CursorGrabMode;
        self.captured = captured;
        if captured {
            // Try Locked first (ideal for FPS), fall back to Confined.
            if window.set_cursor_grab(CursorGrabMode::Locked).is_err() {
                let _ = window.set_cursor_grab(CursorGrabMode::Confined);
            }
            window.set_cursor_visible(false);
        } else {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
        }
    }

    /// Set captured flag without a window reference (for testing).
    #[cfg(test)]
    pub(crate) fn set_captured_flag(&mut self, captured: bool) {
        self.captured = captured;
    }

    /// Clears per-frame transients: delta, scroll, just_pressed, just_released.
    pub fn clear_transients(&mut self) {
        self.prev_position = self.position;
        self.delta = Vec2::ZERO;
        self.scroll = 0.0;
        for b in &mut self.buttons {
            b.just_pressed = false;
            b.just_released = false;
        }
    }

    // ── Queries ─────────────────────────────────────────────────────

    /// Current cursor position in window-logical coordinates.
    #[must_use]
    pub fn position(&self) -> Vec2 {
        self.position
    }

    /// Movement delta since last frame clear.
    #[must_use]
    pub fn delta(&self) -> Vec2 {
        self.delta
    }

    /// Whether a mouse button is currently held.
    #[must_use]
    pub fn is_button_pressed(&self, button: MouseButton) -> bool {
        self.buttons[button_index(button)].pressed
    }

    /// Whether a mouse button was pressed this frame.
    #[must_use]
    pub fn just_button_pressed(&self, button: MouseButton) -> bool {
        self.buttons[button_index(button)].just_pressed
    }

    /// Whether a mouse button was released this frame.
    #[must_use]
    pub fn just_button_released(&self, button: MouseButton) -> bool {
        self.buttons[button_index(button)].just_released
    }

    /// Scroll wheel delta accumulated this frame (positive = scroll up).
    #[must_use]
    pub fn scroll(&self) -> f32 {
        self.scroll
    }

    /// Whether the cursor is currently captured for FPS-style look.
    #[must_use]
    pub fn is_captured(&self) -> bool {
        self.captured
    }

    /// Whether the cursor is inside the window.
    #[must_use]
    pub fn is_cursor_in_window(&self) -> bool {
        self.cursor_in_window
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_position_updates_on_move() {
        let mut ms = MouseState::new();
        ms.on_cursor_moved(100.0, 200.0);
        assert_eq!(ms.position(), Vec2::new(100.0, 200.0));
    }

    #[test]
    fn test_delta_is_difference_between_frames() {
        let mut ms = MouseState::new();
        ms.on_cursor_moved(100.0, 200.0);
        ms.clear_transients();
        ms.on_cursor_moved(110.0, 195.0);
        let d = ms.delta();
        assert!((d.x - 10.0).abs() < f32::EPSILON);
        assert!((d.y - (-5.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn test_button_press_and_release_tracked() {
        let mut ms = MouseState::new();
        ms.on_button(MouseButton::Left, ElementState::Pressed);
        assert!(ms.is_button_pressed(MouseButton::Left));
        assert!(ms.just_button_pressed(MouseButton::Left));

        ms.on_button(MouseButton::Left, ElementState::Released);
        assert!(!ms.is_button_pressed(MouseButton::Left));
        assert!(ms.just_button_released(MouseButton::Left));
    }

    #[test]
    fn test_scroll_accumulates_within_frame() {
        let mut ms = MouseState::new();
        ms.on_scroll(MouseScrollDelta::LineDelta(0.0, 1.0));
        ms.on_scroll(MouseScrollDelta::LineDelta(0.0, 0.5));
        assert!((ms.scroll() - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_scroll_resets_after_clear() {
        let mut ms = MouseState::new();
        ms.on_scroll(MouseScrollDelta::LineDelta(0.0, 1.0));
        ms.clear_transients();
        assert!((ms.scroll()).abs() < f32::EPSILON);
    }

    #[test]
    fn test_cursor_capture_sets_flag() {
        let mut ms = MouseState::new();
        ms.set_captured_flag(true);
        assert!(ms.is_captured());
    }

    #[test]
    fn test_cursor_enter_leave() {
        let mut ms = MouseState::new();
        ms.on_cursor_entered();
        assert!(ms.is_cursor_in_window());
        ms.on_cursor_left();
        assert!(!ms.is_cursor_in_window());
    }

    #[test]
    fn test_delta_resets_each_frame() {
        let mut ms = MouseState::new();
        ms.on_cursor_moved(50.0, 50.0);
        ms.clear_transients();
        assert_eq!(ms.delta(), Vec2::ZERO);
    }
}
