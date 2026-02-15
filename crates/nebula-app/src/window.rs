//! Window creation and event handling via winit.
//!
//! Provides [`AppState`] which implements winit's [`ApplicationHandler`] trait,
//! and a [`run`] function to start the event loop.

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

/// Default window width in logical pixels.
pub const DEFAULT_WIDTH: f64 = 1280.0;
/// Default window height in logical pixels.
pub const DEFAULT_HEIGHT: f64 = 720.0;
/// Default window title.
pub const DEFAULT_TITLE: &str = "Nebula Engine";

/// Returns the default [`WindowAttributes`] for the engine window.
pub fn default_window_attributes() -> WindowAttributes {
    WindowAttributes::default()
        .with_title(DEFAULT_TITLE)
        .with_inner_size(winit::dpi::LogicalSize::new(DEFAULT_WIDTH, DEFAULT_HEIGHT))
}

/// Application state that manages the window and tracks surface dimensions.
pub struct AppState {
    /// The window handle, wrapped in `Arc` for sharing with the renderer.
    pub window: Option<Arc<Window>>,
    /// Current surface width in physical pixels.
    pub surface_width: u32,
    /// Current surface height in physical pixels.
    pub surface_height: u32,
}

impl AppState {
    /// Creates a new `AppState` with default dimensions and no window.
    pub fn new() -> Self {
        Self {
            window: None,
            surface_width: DEFAULT_WIDTH as u32,
            surface_height: DEFAULT_HEIGHT as u32,
        }
    }

    /// Computes the logical size from the current physical size and scale factor.
    pub fn logical_size(&self) -> (f64, f64) {
        let scale = self
            .window
            .as_ref()
            .map(|w| w.scale_factor())
            .unwrap_or(1.0);
        (
            self.surface_width as f64 / scale,
            self.surface_height as f64 / scale,
        )
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl ApplicationHandler for AppState {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let attrs = default_window_attributes();
            let window = event_loop
                .create_window(attrs)
                .expect("Failed to create window");
            self.window = Some(Arc::new(window));
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                log::info!("Close requested, shutting down");
                event_loop.exit();
            }
            WindowEvent::Resized(new_size) => {
                self.surface_width = new_size.width;
                self.surface_height = new_size.height;
                log::info!("Window resized to {}x{}", new_size.width, new_size.height);
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                log::info!("Scale factor changed");
            }
            WindowEvent::RedrawRequested => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

/// Creates an event loop and runs the application.
///
/// This function blocks until the window is closed.
pub fn run() {
    let event_loop = EventLoop::new().expect("Failed to create event loop");
    let mut app = AppState::new();
    event_loop.run_app(&mut app).expect("Event loop failed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_dimensions() {
        let state = AppState::new();
        assert_eq!(state.surface_width, 1280);
        assert_eq!(state.surface_height, 720);
    }

    #[test]
    fn test_app_state_default() {
        let state = AppState::new();
        assert!(state.window.is_none());
    }

    #[test]
    fn test_resize_tracking() {
        let mut state = AppState::new();
        state.surface_width = 1920;
        state.surface_height = 1080;
        assert_eq!(state.surface_width, 1920);
        assert_eq!(state.surface_height, 1080);
    }

    #[test]
    fn test_logical_size_calculation() {
        let mut state = AppState::new();
        // Simulate 2x HiDPI: physical 2560x1440, no window so scale=1.0
        // With no window, scale defaults to 1.0, so logical == physical.
        state.surface_width = 2560;
        state.surface_height = 1440;
        let (lw, lh) = state.logical_size();
        // Without a real window, scale factor is 1.0
        assert!((lw - 2560.0).abs() < f64::EPSILON);
        assert!((lh - 1440.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_window_builder_defaults() {
        let _attrs = default_window_attributes();
        // WindowAttributes doesn't expose getters, so we verify it doesn't panic.
        // The actual size/title are validated by the integration (demo).
    }

    #[test]
    fn test_window_title() {
        assert_eq!(DEFAULT_TITLE, "Nebula Engine");
    }
}
