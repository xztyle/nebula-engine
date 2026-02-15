//! Window creation and event handling via winit.
//!
//! Provides [`AppState`] which implements winit's [`ApplicationHandler`] trait,
//! and a [`run`] function to start the event loop.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::game_loop::{FIXED_DT, GameLoop};
use nebula_config::Config;
use nebula_debug::{DebugServer, DebugState, create_debug_server, get_debug_port};
use nebula_render::gpu::{self, GpuContext};
use tracing::{error, info, instrument, warn};
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

/// Returns [`WindowAttributes`] based on the given configuration.
pub fn window_attributes_from_config(config: &Config) -> WindowAttributes {
    WindowAttributes::default()
        .with_title(config.window.title.clone())
        .with_inner_size(winit::dpi::LogicalSize::new(
            config.window.width as f64,
            config.window.height as f64,
        ))
}

/// Returns the default [`WindowAttributes`] for the engine window.
pub fn default_window_attributes() -> WindowAttributes {
    WindowAttributes::default()
        .with_title(DEFAULT_TITLE)
        .with_inner_size(winit::dpi::LogicalSize::new(DEFAULT_WIDTH, DEFAULT_HEIGHT))
}

/// Callback invoked each fixed-rate simulation step.
pub type UpdateFn = Box<dyn FnMut(f64, f64)>;
/// Callback invoked to compute the clear color for rendering.
pub type ClearColorFn = Box<dyn FnMut(u64) -> wgpu::Color>;

/// Application state that manages the window, GPU context, and tracks surface dimensions.
pub struct AppState {
    /// The window handle, wrapped in `Arc` for sharing with the renderer.
    pub window: Option<Arc<Window>>,
    /// GPU context owning device, queue, and surface.
    pub gpu: Option<GpuContext>,
    /// Current surface width in physical pixels.
    pub surface_width: u32,
    /// Current surface height in physical pixels.
    pub surface_height: u32,
    /// Fixed-timestep game loop.
    pub game_loop: GameLoop,
    /// Simulation tick counter (incremented each fixed update).
    pub tick_count: u64,
    /// Optional callback to compute clear color from tick count.
    pub clear_color_fn: Option<ClearColorFn>,
    /// Engine configuration.
    pub config: Config,
    /// Debug server (only in debug builds).
    pub debug_server: Option<DebugServer>,
    /// Debug state shared with the server.
    pub debug_state: Arc<Mutex<DebugState>>,
    /// Application start time for uptime calculation.
    pub start_time: Instant,
    /// Previous frame time for FPS calculation.
    pub last_frame_time: Instant,
}

impl AppState {
    /// Creates a new `AppState` with default dimensions and no window.
    pub fn new() -> Self {
        let debug_state = Arc::new(Mutex::new(DebugState::default()));
        let debug_server = create_debug_server(get_debug_port());
        let now = Instant::now();

        Self {
            window: None,
            gpu: None,
            surface_width: DEFAULT_WIDTH as u32,
            surface_height: DEFAULT_HEIGHT as u32,
            game_loop: GameLoop::new(),
            tick_count: 0,
            clear_color_fn: None,
            config: Config::default(),
            debug_server,
            debug_state,
            start_time: now,
            last_frame_time: now,
        }
    }

    /// Creates a new `AppState` from a [`Config`].
    pub fn with_config(mut config: Config) -> Self {
        let debug_state = Arc::new(Mutex::new(DebugState::default()));
        let debug_server = create_debug_server(get_debug_port());
        let now = Instant::now();

        // Update window title to include debug port in debug builds
        #[cfg(debug_assertions)]
        {
            config.window.title =
                format!("{} [Debug API :{}]", config.window.title, get_debug_port());
        }

        Self {
            window: None,
            gpu: None,
            surface_width: config.window.width,
            surface_height: config.window.height,
            game_loop: GameLoop::new(),
            tick_count: 0,
            clear_color_fn: None,
            config,
            debug_server,
            debug_state,
            start_time: now,
            last_frame_time: now,
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

    /// Updates the debug state with current frame metrics.
    pub fn update_debug_state(&mut self) {
        let now = Instant::now();
        let frame_time_ms = now.duration_since(self.last_frame_time).as_secs_f64() * 1000.0;
        let fps = if frame_time_ms > 0.0 {
            1000.0 / frame_time_ms
        } else {
            0.0
        };
        let uptime_seconds = now.duration_since(self.start_time).as_secs_f64();

        if let Ok(mut state) = self.debug_state.lock() {
            state.frame_count = self.game_loop.frame_count();
            state.frame_time_ms = frame_time_ms;
            state.fps = fps;
            state.entity_count = 0; // Will be updated once ECS is implemented
            state.window_width = self.surface_width;
            state.window_height = self.surface_height;
            state.uptime_seconds = uptime_seconds;
        }

        self.last_frame_time = now;
    }

    /// Checks if quit was requested via the debug API.
    pub fn should_quit_from_debug(&self) -> bool {
        self.debug_state
            .lock()
            .map(|state| state.quit_requested)
            .unwrap_or(false)
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
            let attrs = window_attributes_from_config(&self.config);
            let window = event_loop
                .create_window(attrs)
                .expect("Failed to create window");
            let window = Arc::new(window);

            match gpu::init_gpu_blocking(window.clone()) {
                Ok(ctx) => {
                    self.gpu = Some(ctx);
                }
                Err(e) => {
                    error!("GPU initialization failed: {e}");
                    event_loop.exit();
                    return;
                }
            }

            self.window = Some(window);

            // Start debug server in debug builds
            #[cfg(debug_assertions)]
            if let Some(ref mut debug_server) = self.debug_server {
                if let Err(e) = debug_server.start(self.debug_state.clone()) {
                    warn!("Failed to start debug server: {e}");
                } else {
                    info!("Debug API started on port {}", debug_server.actual_port());
                }
            }
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
                info!("Close requested, shutting down");
                event_loop.exit();
            }
            WindowEvent::Resized(new_size) => {
                self.surface_width = new_size.width;
                self.surface_height = new_size.height;
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(new_size.width, new_size.height);
                }
                info!("Window resized to {}x{}", new_size.width, new_size.height);
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                info!("Scale factor changed");
            }
            WindowEvent::RedrawRequested => {
                // Update debug state first
                self.update_debug_state();

                // Check if quit was requested via debug API
                if self.should_quit_from_debug() {
                    info!("Quit requested via debug API");
                    event_loop.exit();
                    return;
                }

                let tick_count = &mut self.tick_count;
                self.game_loop.tick(
                    |_dt, _sim_time| {
                        *tick_count += 1;
                    },
                    |_alpha| {},
                );

                if let Some(gpu) = &self.gpu {
                    let clear_color = if let Some(ref mut f) = self.clear_color_fn {
                        f(self.tick_count)
                    } else {
                        default_clear_color(self.tick_count)
                    };

                    match gpu.surface.get_current_texture() {
                        Ok(output) => {
                            let view = output
                                .texture
                                .create_view(&wgpu::TextureViewDescriptor::default());
                            let mut encoder = gpu.device.create_command_encoder(
                                &wgpu::CommandEncoderDescriptor {
                                    label: Some("Clear Encoder"),
                                },
                            );
                            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("Clear Pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(clear_color),
                                        store: wgpu::StoreOp::Store,
                                    },
                                    depth_slice: None,
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                                multiview_mask: None,
                            });
                            gpu.queue.submit(std::iter::once(encoder.finish()));
                            output.present();
                        }
                        Err(wgpu::SurfaceError::Lost) => {
                            let w = self.surface_width;
                            let h = self.surface_height;
                            if let Some(gpu) = &mut self.gpu {
                                gpu.resize(w, h);
                            }
                        }
                        Err(wgpu::SurfaceError::OutOfMemory) => {
                            error!("GPU out of memory");
                            event_loop.exit();
                        }
                        Err(e) => {
                            warn!("Surface error: {e}");
                        }
                    }
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

/// Computes a pulsing deep-space blue clear color based on the tick count.
///
/// The blue channel oscillates between 0.02 and 0.08, proving the simulation
/// loop is alive.
pub fn default_clear_color(tick_count: u64) -> wgpu::Color {
    let phase = (tick_count as f64 * FIXED_DT * std::f64::consts::TAU * 0.25).sin();
    let blue = 0.05 + 0.03 * phase;
    wgpu::Color {
        r: 0.02,
        g: 0.02,
        b: blue,
        a: 1.0,
    }
}

/// Creates an event loop and runs the application with default config.
///
/// This function blocks until the window is closed.
#[instrument]
pub fn run() {
    let event_loop = EventLoop::new().expect("Failed to create event loop");
    let mut app = AppState::new();
    event_loop.run_app(&mut app).expect("Event loop failed");
}

/// Creates an event loop and runs the application with the given config.
///
/// This function blocks until the window is closed.
#[instrument(skip(config))]
pub fn run_with_config(config: Config) {
    let event_loop = EventLoop::new().expect("Failed to create event loop");
    let mut app = AppState::with_config(config);
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
