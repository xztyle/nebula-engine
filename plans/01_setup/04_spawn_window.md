# Spawn Window

## Problem

The engine needs a visible window on all platforms to serve as the rendering surface. Without a window, there is no surface for wgpu to present frames to, no way to receive input events, and no visible application. The window must:

- Open reliably on Linux (X11 and Wayland), Windows, and macOS.
- Handle resize events so the renderer can recreate its swap chain at the new dimensions.
- Handle close events so the application exits gracefully without leaking resources.
- Handle DPI scaling so the window looks correct on HiDPI/Retina displays.
- Be shareable with the rendering subsystem (which needs a raw window handle for surface creation).
- Integrate with the event loop that will also drive input, game logic, and rendering.

Winit is the de facto standard for cross-platform windowing in the Rust ecosystem. It handles the platform differences internally (X11/Wayland/Win32/Cocoa) and exposes a unified event loop API.

## Solution

### Window Creation

Use `winit` to create an event loop and window:

```rust
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes};
use std::sync::Arc;

pub struct AppState {
    window: Option<Arc<Window>>,
    surface_width: u32,
    surface_height: u32,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            window: None,
            surface_width: 1280,
            surface_height: 720,
        }
    }
}

impl ApplicationHandler for AppState {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let attrs = WindowAttributes::default()
                .with_title("Nebula Engine")
                .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));

            let window = event_loop
                .create_window(attrs)
                .expect("Failed to create window");

            self.window = Some(Arc::new(window));
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
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
                log::info!(
                    "Window resized to {}x{}",
                    new_size.width,
                    new_size.height
                );
                // Renderer will reconfigure surface on next frame
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                // Handle DPI changes (e.g., moving window between monitors)
                log::info!("Scale factor changed");
            }
            WindowEvent::RedrawRequested => {
                // Trigger frame rendering here
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

pub fn run() {
    let event_loop = EventLoop::new()
        .expect("Failed to create event loop");
    let mut app = AppState::new();
    event_loop.run_app(&mut app)
        .expect("Event loop failed");
}
```

### Key Design Decisions

1. **`Arc<Window>`** — The window is wrapped in `Arc` because the renderer needs a reference to it for creating the wgpu surface. The renderer and the event loop handler both need access to the window, and `Arc` provides safe shared ownership.

2. **Logical size 1280x720** — Using `LogicalSize` instead of `PhysicalSize` means the window is 1280x720 in logical (DPI-independent) units. On a 2x HiDPI display, the physical size will be 2560x1440, which is what the renderer needs to know about.

3. **`ApplicationHandler` trait** — Winit 0.30 uses the `ApplicationHandler` trait pattern instead of the closure-based `run` method. This provides better structure and makes the event handling logic easier to test.

4. **`RedrawRequested` loop** — The window requests a redraw after each frame. This creates a continuous rendering loop driven by the OS's display refresh when vsync is enabled. When the game loop is integrated (see `06_main_game_loop_with_fixed_timestep.md`), this will be the point where frame rendering is triggered.

5. **Resize tracking** — The current surface dimensions are stored in `AppState` so the renderer can query them when reconfiguring the swap chain. The renderer does not reconfigure immediately during the resize event to avoid redundant reconfiguration during drag-resize (which fires many resize events rapidly).

### DPI Handling

DPI scaling is handled transparently by winit. The `PhysicalSize` returned by resize events is always in physical pixels, which is what wgpu needs for its surface configuration. The logical size (used for UI layout) can be computed by dividing physical size by the scale factor.

```rust
pub fn logical_size(&self) -> (f64, f64) {
    let scale = self.window.as_ref()
        .map(|w| w.scale_factor())
        .unwrap_or(1.0);
    (
        self.surface_width as f64 / scale,
        self.surface_height as f64 / scale,
    )
}
```

### Platform Notes

- **Linux (Wayland)** — Winit defaults to Wayland if available, falling back to X11. Both are supported without engine code changes.
- **Windows** — Win32 window creation is handled internally by winit. No special configuration needed.
- **macOS** — The event loop must run on the main thread (an OS requirement). Winit enforces this automatically.

## Outcome

Running the application opens a window titled "Nebula Engine" at 1280x720 logical pixels. The window is centered on the primary monitor (winit default behavior). Clicking the close button logs a message and exits the process cleanly. Resizing the window updates the internal `surface_width` and `surface_height` fields. The window handle is available as an `Arc<Window>` for the renderer to create a wgpu surface.

## Demo Integration

**Demo crate:** `nebula-demo`

The demo opens a 1280x720 window titled "Nebula Engine" with a black background. The window is resizable and closeable. This is the first time a human can see the demo do something.

## Crates & Dependencies

- **`winit = "0.30"`** — Cross-platform window management. Provides the event loop, window creation, and input event handling. Supports Linux (X11/Wayland), Windows (Win32), and macOS (Cocoa/AppKit).
- **`log = "0.4"`** — Logging facade. Used for diagnostic messages during window events. The actual log output is configured by `nebula-debug` (see `08_logging_and_tracing.md`).

## Unit Tests

- **`test_window_builder_defaults`** — Create a `WindowAttributes` with the engine's default configuration and verify that the requested inner size is 1280x720 logical pixels. This test does not require a display server; it only tests the configuration values.

- **`test_window_title`** — Create a `WindowAttributes` with the engine's defaults and verify the title is set to `"Nebula Engine"`. This validates that the title configuration is not accidentally overwritten.

- **`test_resize_tracking`** — Create an `AppState`, manually set `surface_width` and `surface_height` to simulate a resize event, and verify the new dimensions are stored correctly. This tests the state management without needing a real window.

- **`test_initial_dimensions`** — Create a new `AppState` and verify that `surface_width` is 1280 and `surface_height` is 720 before any events are processed.

- **`test_logical_size_calculation`** — Given a physical size of 2560x1440 and a scale factor of 2.0, verify that the `logical_size()` method returns (1280.0, 720.0). This validates DPI handling math.

- **`test_app_state_default`** — Create a new `AppState` and verify that `window` is `None` (the window is created lazily on the `resumed` event, not at construction time).
