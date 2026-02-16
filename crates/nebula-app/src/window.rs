//! Window creation and event handling via winit.
//!
//! Provides [`AppState`] which implements winit's [`ApplicationHandler`] trait,
//! and a [`run`] function to start the event loop.

use glam;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::game_loop::GameLoop;
use bytemuck;
use nebula_config::Config;
use nebula_debug::{DebugServer, DebugState, create_debug_server, get_debug_port};
use nebula_render::{
    BufferAllocator, Camera, DepthBuffer, FrameEncoder, IndexData, MeshBuffer, RenderContext,
    RenderPassBuilder, ShaderLibrary, SurfaceWrapper, TEXTURED_SHADER_SOURCE, TextureManager,
    TexturedPipeline, UNLIT_SHADER_SOURCE, UnlitPipeline, VertexPositionColor,
    VertexPositionNormalUv, draw_textured, draw_unlit, init_render_context_blocking,
};
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
/// Custom update function that gets called each simulation tick.
pub type CustomUpdateFn = Box<dyn FnMut(f64)>;

/// Application state that manages the window, GPU context, and tracks surface dimensions.
pub struct AppState {
    /// The window handle, wrapped in `Arc` for sharing with the renderer.
    pub window: Option<Arc<Window>>,
    /// GPU context owning device, queue, and surface.
    pub gpu: Option<RenderContext>,
    /// Cross-platform surface wrapper that normalizes resize/DPI behavior.
    pub surface_wrapper: SurfaceWrapper,
    /// Fixed-timestep game loop.
    pub game_loop: GameLoop,
    /// Simulation tick counter (incremented each fixed update).
    pub tick_count: u64,
    /// Optional callback to compute clear color from tick count.
    pub clear_color_fn: Option<ClearColorFn>,
    /// Optional custom update function called each simulation tick.
    pub custom_update: Option<CustomUpdateFn>,
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
    /// Unlit rendering pipeline.
    pub unlit_pipeline: Option<UnlitPipeline>,
    /// Triangle mesh for rendering.
    pub triangle_mesh: Option<MeshBuffer>,
    /// Second triangle mesh behind the first for depth testing.
    pub back_triangle_mesh: Option<MeshBuffer>,
    /// Depth buffer for 3D depth testing.
    pub depth_buffer: Option<DepthBuffer>,
    /// Camera uniform buffer.
    pub camera_buffer: Option<wgpu::Buffer>,
    /// Camera bind group.
    pub camera_bind_group: Option<wgpu::BindGroup>,
    /// 3D camera for rendering.
    pub camera: Camera,
    /// Time accumulator for camera animation.
    pub camera_time: f64,
    /// Textured pipeline for the checkerboard quad.
    pub textured_pipeline: Option<TexturedPipeline>,
    /// Textured quad mesh.
    pub textured_quad_mesh: Option<MeshBuffer>,
    /// Managed checkerboard texture (owns the bind group).
    pub checkerboard_texture: Option<std::sync::Arc<nebula_render::ManagedTexture>>,
    /// Camera bind group for the textured pipeline.
    pub textured_camera_bind_group: Option<wgpu::BindGroup>,
    /// Cube-face quad meshes (six faces, each a distinct color).
    pub cube_face_meshes: Vec<MeshBuffer>,
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
            surface_wrapper: SurfaceWrapper::new(DEFAULT_WIDTH as u32, DEFAULT_HEIGHT as u32, 1.0),
            game_loop: GameLoop::new(),
            tick_count: 0,
            clear_color_fn: None,
            custom_update: None,
            config: Config::default(),
            debug_server,
            debug_state,
            start_time: now,
            last_frame_time: now,
            unlit_pipeline: None,
            triangle_mesh: None,
            back_triangle_mesh: None,
            depth_buffer: None,
            camera_buffer: None,
            camera_bind_group: None,
            camera: Camera::default(),
            camera_time: 0.0,
            textured_pipeline: None,
            textured_quad_mesh: None,
            checkerboard_texture: None,
            textured_camera_bind_group: None,
            cube_face_meshes: Vec::new(),
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
            surface_wrapper: SurfaceWrapper::new(config.window.width, config.window.height, 1.0),
            game_loop: GameLoop::new(),
            tick_count: 0,
            clear_color_fn: None,
            custom_update: None,
            config,
            debug_server,
            debug_state,
            start_time: now,
            last_frame_time: now,
            unlit_pipeline: None,
            triangle_mesh: None,
            back_triangle_mesh: None,
            depth_buffer: None,
            camera_buffer: None,
            camera_bind_group: None,
            camera: Camera::default(),
            camera_time: 0.0,
            textured_pipeline: None,
            textured_quad_mesh: None,
            checkerboard_texture: None,
            textured_camera_bind_group: None,
            cube_face_meshes: Vec::new(),
        }
    }

    /// Computes the logical size from the current physical size and scale factor.
    pub fn logical_size(&self) -> (f64, f64) {
        (
            self.surface_wrapper.logical_width(),
            self.surface_wrapper.logical_height(),
        )
    }

    /// Returns the current physical surface width.
    pub fn surface_width(&self) -> u32 {
        self.surface_wrapper.physical_width()
    }

    /// Returns the current physical surface height.
    pub fn surface_height(&self) -> u32 {
        self.surface_wrapper.physical_height()
    }

    /// Initialize the rendering pipeline and resources.
    fn initialize_rendering(&mut self, gpu: &RenderContext) {
        use wgpu::util::DeviceExt;

        // Create depth buffer
        let depth_buffer =
            DepthBuffer::new(&gpu.device, self.surface_width(), self.surface_height());

        // Load the unlit shader
        let mut shader_library = ShaderLibrary::new();
        let shader = shader_library
            .load_from_source(&gpu.device, "unlit", UNLIT_SHADER_SOURCE)
            .expect("Failed to load unlit shader");

        // Create the unlit pipeline with depth testing enabled
        let unlit_pipeline = UnlitPipeline::new(
            &gpu.device,
            &shader,
            gpu.surface_format,
            Some(DepthBuffer::FORMAT), // enable depth testing
        );

        // Create front triangle mesh (closer to camera)
        let front_vertices = [
            VertexPositionColor {
                position: [0.0, 0.5, 0.0],
                color: [1.0, 0.0, 0.0, 1.0],
            }, // red top
            VertexPositionColor {
                position: [-0.5, -0.5, 0.0],
                color: [0.0, 1.0, 0.0, 1.0],
            }, // green left
            VertexPositionColor {
                position: [0.5, -0.5, 0.0],
                color: [0.0, 0.0, 1.0, 1.0],
            }, // blue right
        ];

        // Create back triangle mesh (farther from camera, partially overlapping)
        let back_vertices = [
            VertexPositionColor {
                position: [0.25, 0.25, -1.0], // offset to the right and back
                color: [1.0, 1.0, 0.0, 1.0],  // yellow top
            },
            VertexPositionColor {
                position: [-0.25, -0.75, -1.0],
                color: [0.0, 1.0, 1.0, 1.0], // cyan left
            },
            VertexPositionColor {
                position: [0.75, -0.75, -1.0],
                color: [1.0, 0.0, 1.0, 1.0], // magenta right
            },
        ];

        let indices: [u16; 3] = [0, 1, 2];

        let allocator = BufferAllocator::new(&gpu.device);
        let triangle_mesh = allocator.create_mesh(
            "front-triangle",
            bytemuck::cast_slice(&front_vertices),
            IndexData::U16(&indices),
        );

        let back_triangle_mesh = allocator.create_mesh(
            "back-triangle",
            bytemuck::cast_slice(&back_vertices),
            IndexData::U16(&indices),
        );

        // Initialize camera position (orbit around the triangle at distance 3)
        self.camera.position = glam::Vec3::new(0.0, 0.0, 3.0);
        self.camera
            .set_aspect_ratio(self.surface_width() as f32, self.surface_height() as f32);

        // Create camera uniform buffer with camera's view-projection matrix
        let camera_uniform = self.camera.to_uniform();

        let camera_buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("camera-uniform"),
                contents: bytemuck::cast_slice(&[camera_uniform]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

        // Create camera bind group
        let camera_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera-bind-group"),
            layout: &unlit_pipeline.camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        // --- Textured checkerboard quad ---
        let mut texture_manager = TextureManager::new(&gpu.device);
        let checkerboard_data = generate_checkerboard(64, 64, 8);
        let managed_tex = texture_manager
            .create_texture(
                &gpu.device,
                &gpu.queue,
                "checkerboard",
                &checkerboard_data,
                64,
                64,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                true,
            )
            .expect("Failed to create checkerboard texture");

        let textured_shader = shader_library
            .load_from_source(&gpu.device, "textured", TEXTURED_SHADER_SOURCE)
            .expect("Failed to load textured shader");

        let textured_pipeline = TexturedPipeline::new(
            &gpu.device,
            &textured_shader,
            gpu.surface_format,
            Some(DepthBuffer::FORMAT),
            texture_manager.bind_group_layout(),
        );

        // Quad behind the triangles at z = -2
        let quad_vertices = [
            VertexPositionNormalUv {
                position: [-1.5, -1.5, -2.0],
                normal: [0.0, 0.0, 1.0],
                uv: [0.0, 1.0],
            },
            VertexPositionNormalUv {
                position: [1.5, -1.5, -2.0],
                normal: [0.0, 0.0, 1.0],
                uv: [1.0, 1.0],
            },
            VertexPositionNormalUv {
                position: [1.5, 1.5, -2.0],
                normal: [0.0, 0.0, 1.0],
                uv: [1.0, 0.0],
            },
            VertexPositionNormalUv {
                position: [-1.5, 1.5, -2.0],
                normal: [0.0, 0.0, 1.0],
                uv: [0.0, 0.0],
            },
        ];
        let quad_indices: [u16; 6] = [0, 1, 2, 2, 3, 0];

        let textured_quad_mesh = allocator.create_mesh(
            "checkerboard-quad",
            bytemuck::cast_slice(&quad_vertices),
            IndexData::U16(&quad_indices),
        );

        let textured_camera_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("textured-camera-bind-group"),
            layout: &textured_pipeline.camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        self.textured_pipeline = Some(textured_pipeline);
        self.textured_quad_mesh = Some(textured_quad_mesh);
        self.checkerboard_texture = Some(managed_tex);
        self.textured_camera_bind_group = Some(textured_camera_bind_group);

        self.unlit_pipeline = Some(unlit_pipeline);
        self.triangle_mesh = Some(triangle_mesh);
        self.back_triangle_mesh = Some(back_triangle_mesh);
        self.depth_buffer = Some(depth_buffer);
        self.camera_buffer = Some(camera_buffer);
        self.camera_bind_group = Some(camera_bind_group);

        // Create cube-face quads using CubeFace basis vectors
        self.cube_face_meshes = create_cube_face_meshes(&allocator);

        info!(
            "Rendering pipeline initialized successfully with depth buffer, textured quad, and cube faces"
        );
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
            state.window_width = self.surface_width();
            state.window_height = self.surface_height();
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

            // Initialize the surface wrapper with actual window dimensions and scale
            let scale_factor = window.scale_factor();
            let inner_size = window.inner_size();
            self.surface_wrapper =
                SurfaceWrapper::new(inner_size.width, inner_size.height, scale_factor);
            info!(
                "Surface wrapper initialized: {}x{} (scale: {:.2})",
                inner_size.width, inner_size.height, scale_factor
            );

            match init_render_context_blocking(window.clone()) {
                Ok(ctx) => {
                    // Initialize rendering pipeline and resources
                    self.initialize_rendering(&ctx);
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
                if let Some(resize) = self
                    .surface_wrapper
                    .handle_resize(new_size.width, new_size.height)
                {
                    let w = resize.physical.width;
                    let h = resize.physical.height;

                    // Update camera aspect ratio
                    self.camera.set_aspect_ratio(w as f32, h as f32);

                    if let Some(gpu) = &mut self.gpu {
                        gpu.resize(w, h);
                    }

                    // Resize depth buffer
                    if let (Some(depth_buffer), Some(gpu)) = (&mut self.depth_buffer, &self.gpu) {
                        depth_buffer.resize(&gpu.device, w, h);
                    }

                    info!(
                        "Window resized to {}x{} (scale: {:.2})",
                        w, h, resize.scale_factor
                    );
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // Get the new physical size from the window after the scale change
                if let Some(window) = &self.window {
                    let new_inner = window.inner_size();
                    if let Some(resize) = self.surface_wrapper.handle_scale_factor_changed(
                        scale_factor,
                        new_inner.width,
                        new_inner.height,
                    ) {
                        let w = resize.physical.width;
                        let h = resize.physical.height;

                        self.camera.set_aspect_ratio(w as f32, h as f32);

                        if let Some(gpu) = &mut self.gpu {
                            gpu.resize(w, h);
                        }

                        if let (Some(depth_buffer), Some(gpu)) = (&mut self.depth_buffer, &self.gpu)
                        {
                            depth_buffer.resize(&gpu.device, w, h);
                        }

                        info!(
                            "Scale factor changed to {:.2}, resized to {}x{}",
                            scale_factor, w, h
                        );
                    }
                }
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
                let custom_update = &mut self.custom_update;
                let camera = &mut self.camera;
                let camera_time = &mut self.camera_time;
                let camera_buffer = &self.camera_buffer;
                let gpu = &self.gpu;

                self.game_loop.tick(
                    |dt, _sim_time| {
                        *tick_count += 1;
                        *camera_time += dt;

                        // Update camera to orbit around the triangle
                        let orbit_radius = 3.0f64;
                        let orbit_speed = 0.5f64; // radians per second
                        let angle = *camera_time * orbit_speed;

                        // Position camera on a circular orbit around the origin
                        camera.position = glam::Vec3::new(
                            (angle.cos() * orbit_radius) as f32,
                            0.0,
                            (angle.sin() * orbit_radius) as f32,
                        );

                        // Make camera look at the origin (triangle center)
                        let target = glam::Vec3::ZERO;
                        let up = glam::Vec3::Y;
                        let forward = (target - camera.position).normalize();
                        let right = forward.cross(up).normalize();
                        let camera_up = right.cross(forward).normalize();

                        // Create rotation matrix from basis vectors
                        let rotation_mat = glam::Mat3::from_cols(right, camera_up, -forward);
                        camera.rotation = glam::Quat::from_mat3(&rotation_mat);

                        // Update camera uniform buffer
                        if let (Some(buffer), Some(gpu)) = (camera_buffer, gpu) {
                            let uniform = camera.to_uniform();
                            gpu.queue
                                .write_buffer(buffer, 0, bytemuck::cast_slice(&[uniform]));
                        }

                        // Call custom update function if provided
                        if let Some(update_fn) = custom_update {
                            update_fn(dt);
                        }
                    },
                    |_alpha| {},
                );

                if let Some(gpu) = &self.gpu {
                    let clear_color = if let Some(ref mut f) = self.clear_color_fn {
                        f(self.tick_count)
                    } else {
                        default_clear_color(self.tick_count)
                    };

                    match gpu.get_current_texture() {
                        Ok(surface_texture) => {
                            let mut frame_encoder = FrameEncoder::new(
                                &gpu.device,
                                Arc::new(gpu.queue.clone()),
                                surface_texture,
                            );

                            // Create render pass builder with depth buffer
                            let pass_builder = if let Some(depth_buffer) = &self.depth_buffer {
                                RenderPassBuilder::new()
                                    .clear_color(clear_color)
                                    .depth(depth_buffer.view.clone(), DepthBuffer::CLEAR_VALUE)
                                    .label("depth-test-pass")
                            } else {
                                RenderPassBuilder::new().clear_color(clear_color)
                            };

                            {
                                let mut render_pass =
                                    frame_encoder.begin_render_pass(&pass_builder);

                                // Render both triangles if we have all the required resources
                                if let (Some(pipeline), Some(bind_group)) =
                                    (&self.unlit_pipeline, &self.camera_bind_group)
                                {
                                    // Render the back triangle first (farther away)
                                    if let Some(back_mesh) = &self.back_triangle_mesh {
                                        draw_unlit(
                                            &mut render_pass,
                                            pipeline,
                                            bind_group,
                                            back_mesh,
                                        );
                                    }

                                    // Render the front triangle second (closer)
                                    if let Some(front_mesh) = &self.triangle_mesh {
                                        draw_unlit(
                                            &mut render_pass,
                                            pipeline,
                                            bind_group,
                                            front_mesh,
                                        );
                                    }
                                }

                                // Render the textured checkerboard quad behind everything
                                if let (
                                    Some(tex_pipeline),
                                    Some(tex_cam_bg),
                                    Some(checker_tex),
                                    Some(quad_mesh),
                                ) = (
                                    &self.textured_pipeline,
                                    &self.textured_camera_bind_group,
                                    &self.checkerboard_texture,
                                    &self.textured_quad_mesh,
                                ) {
                                    draw_textured(
                                        &mut render_pass,
                                        tex_pipeline,
                                        tex_cam_bg,
                                        &checker_tex.bind_group,
                                        quad_mesh,
                                    );
                                }

                                // Render cube-face quads
                                if let (Some(pipeline), Some(bind_group)) =
                                    (&self.unlit_pipeline, &self.camera_bind_group)
                                {
                                    for mesh in &self.cube_face_meshes {
                                        draw_unlit(&mut render_pass, pipeline, bind_group, mesh);
                                    }
                                }
                            }

                            // Capture screenshot if requested by the debug API
                            #[cfg(debug_assertions)]
                            let screenshot_readback = if self
                                .debug_state
                                .lock()
                                .map(|s| s.screenshot_requested)
                                .unwrap_or(false)
                            {
                                frame_encoder.copy_surface_to_buffer(&gpu.device)
                            } else {
                                None
                            };

                            frame_encoder.submit();

                            // After submit, map the readback buffer and encode as PNG
                            #[cfg(debug_assertions)]
                            if let Some((readback_buffer, tex_width, tex_height, padded_row)) =
                                screenshot_readback
                            {
                                let bytes_per_pixel = 4u32;
                                let buffer_slice = readback_buffer.slice(..);
                                let (tx, rx) = std::sync::mpsc::channel();
                                buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
                                    let _ = tx.send(result);
                                });
                                let _ = gpu.device.poll(wgpu::PollType::Wait {
                                    submission_index: None,
                                    timeout: None,
                                });

                                if let Ok(Ok(())) = rx.recv() {
                                    let mapped = buffer_slice.get_mapped_range();
                                    let is_bgra = matches!(
                                        gpu.surface_format,
                                        wgpu::TextureFormat::Bgra8Unorm
                                            | wgpu::TextureFormat::Bgra8UnormSrgb
                                    );
                                    let mut pixels =
                                        Vec::with_capacity((tex_width * tex_height * 4) as usize);
                                    for row in 0..tex_height {
                                        let start = (row * padded_row) as usize;
                                        let end = start + (tex_width * bytes_per_pixel) as usize;
                                        let row_data = &mapped[start..end];
                                        if is_bgra {
                                            for chunk in row_data.chunks_exact(4) {
                                                pixels.push(chunk[2]); // R
                                                pixels.push(chunk[1]); // G
                                                pixels.push(chunk[0]); // B
                                                pixels.push(chunk[3]); // A
                                            }
                                        } else {
                                            pixels.extend_from_slice(row_data);
                                        }
                                    }
                                    drop(mapped);

                                    let mut png_buf = Vec::new();
                                    {
                                        let mut encoder = png::Encoder::new(
                                            std::io::Cursor::new(&mut png_buf),
                                            tex_width,
                                            tex_height,
                                        );
                                        encoder.set_color(png::ColorType::Rgba);
                                        encoder.set_depth(png::BitDepth::Eight);
                                        if let Ok(mut writer) = encoder.write_header() {
                                            let _ = writer.write_image_data(&pixels);
                                        }
                                    }

                                    if let Ok(mut state) = self.debug_state.lock() {
                                        state.screenshot_data = Some(png_buf);
                                    }
                                }
                            }
                        }
                        Err(nebula_render::SurfaceError::Lost) => {
                            let size = self.surface_wrapper.physical_size();
                            if let Some(gpu) = &mut self.gpu {
                                gpu.resize(size.width, size.height);
                            }
                        }
                        Err(nebula_render::SurfaceError::OutOfMemory) => {
                            error!("GPU out of memory");
                            event_loop.exit();
                        }
                        Err(nebula_render::SurfaceError::Timeout) => {
                            warn!("Surface timeout, skipping frame");
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

/// Deep space blue clear color as specified in the plan.
///
/// Set to (0.02, 0.02, 0.08) - a steady deep space blue color.
/// The window is now fully GPU-owned — wgpu controls every pixel.
pub fn default_clear_color(_tick_count: u64) -> wgpu::Color {
    wgpu::Color {
        r: 0.02,
        g: 0.02,
        b: 0.08,
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

/// Creates an event loop and runs the application with the given config and custom state.
///
/// This function blocks until the window is closed. The custom state will be updated
/// each simulation tick.
#[instrument(skip_all)]
pub fn run_with_config_and_update<T>(config: Config, mut custom_state: T)
where
    T: FnMut(f64) + 'static,
{
    let event_loop = EventLoop::new().expect("Failed to create event loop");
    let mut app = AppState::with_config(config);

    // Store the custom update function in a Box for the app state
    app.custom_update = Some(Box::new(move |dt: f64| {
        custom_state(dt);
    }));

    event_loop.run_app(&mut app).expect("Event loop failed");
}

/// Create six colored quad meshes, one per [`CubeFace`], arranged as a cube
/// floating at `(0, 0, -5)` with half-extent 0.6.
fn create_cube_face_meshes(allocator: &BufferAllocator) -> Vec<MeshBuffer> {
    use nebula_cubesphere::CubeFace;

    // Colors: PosX=red, NegX=cyan, PosY=green, NegY=magenta, PosZ=blue, NegZ=yellow
    let colors: [[f32; 4]; 6] = [
        [1.0, 0.0, 0.0, 1.0], // PosX - red
        [0.0, 1.0, 1.0, 1.0], // NegX - cyan
        [0.0, 1.0, 0.0, 1.0], // PosY - green
        [1.0, 0.0, 1.0, 1.0], // NegY - magenta
        [0.0, 0.0, 1.0, 1.0], // PosZ - blue
        [1.0, 1.0, 0.0, 1.0], // NegZ - yellow
    ];

    let half = 0.6_f64;
    let center = glam::DVec3::new(0.0, 0.0, -5.0);

    CubeFace::ALL
        .iter()
        .zip(colors.iter())
        .enumerate()
        .map(|(i, (face, color))| {
            let n = face.normal() * half;
            let t = face.tangent() * half;
            let b = face.bitangent() * half;
            let face_center = center + n;

            // Four corners of the quad
            let corners = [
                face_center - t - b, // bottom-left
                face_center + t - b, // bottom-right
                face_center + t + b, // top-right
                face_center - t + b, // top-left
            ];

            let verts: Vec<VertexPositionColor> = corners
                .iter()
                .map(|c| VertexPositionColor {
                    position: [c.x as f32, c.y as f32, c.z as f32],
                    color: *color,
                })
                .collect();

            let indices: [u16; 6] = [0, 1, 2, 2, 3, 0];
            let label = format!("cube-face-{i}");
            allocator.create_mesh(
                &label,
                bytemuck::cast_slice(&verts),
                IndexData::U16(&indices),
            )
        })
        .collect()
}

/// Generate a checkerboard RGBA8 texture.
fn generate_checkerboard(width: u32, height: u32, cell_size: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let is_white = ((x / cell_size) + (y / cell_size)).is_multiple_of(2);
            if is_white {
                data.extend_from_slice(&[230, 230, 230, 255]);
            } else {
                data.extend_from_slice(&[40, 40, 40, 255]);
            }
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_dimensions() {
        let state = AppState::new();
        assert_eq!(state.surface_width(), 1280);
        assert_eq!(state.surface_height(), 720);
    }

    #[test]
    fn test_app_state_default() {
        let state = AppState::new();
        assert!(state.window.is_none());
    }

    #[test]
    fn test_resize_tracking() {
        let mut state = AppState::new();
        state.surface_wrapper.handle_resize(1920, 1080);
        assert_eq!(state.surface_width(), 1920);
        assert_eq!(state.surface_height(), 1080);
    }

    #[test]
    fn test_logical_size_calculation() {
        let mut state = AppState::new();
        // Simulate 2x HiDPI: physical 2560x1440, scale=1.0
        state.surface_wrapper.handle_resize(2560, 1440);
        let (lw, lh) = state.logical_size();
        // Scale factor is 1.0 (default), so logical == physical
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
