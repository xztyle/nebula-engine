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
use nebula_lighting::{
    CascadedShadowConfig, CascadedShadowMaps, DirectionalLight, LightingAtmosphereConfig,
    LightingContext, PointLight, PointLightFrustum, PointLightManager,
    lighting_context_at_altitude, modulate_ambient_by_sun,
};
use nebula_planet::{
    AtmosphereParams, AtmosphereRenderer, DayNightState, ImpostorConfig, ImpostorRenderer,
    ImpostorState, LocalFrustum, OceanParams, OceanRenderer, OrbitalRenderer, OriginManager,
    PlanetFaces, PlanetaryCoord, TransitionConfig, chunk_budget_for_altitude, create_orbit_camera,
    generate_orbital_sphere, impostor_quad_size,
};
use nebula_render::{
    BloomConfig, BloomPipeline, BufferAllocator, Camera, CameraUniform, DepthBuffer, FrameEncoder,
    IndexData, LIT_SHADER_SOURCE, LitPipeline, MeshBuffer, RenderContext, RenderPassBuilder,
    SHADOW_SHADER_SOURCE, ShaderLibrary, ShadowPipeline, SurfaceWrapper, TEXTURED_SHADER_SOURCE,
    TextureManager, TexturedPipeline, UNLIT_SHADER_SOURCE, UnlitPipeline, VertexPositionColor,
    VertexPositionNormalUv, draw_lit, draw_textured, draw_unlit, init_render_context_blocking,
    render_shadow_cascades,
};
use nebula_space::{
    DistantPlanet, ImpostorInstance, NebulaConfig, NebulaGenerator, OrbitalElements,
    PlanetImpostorRenderer, SkyboxRenderer, StarType, StarfieldCubemap, StarfieldGenerator,
    SunProperties, SunRenderer, billboard_local_sun_dir,
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
/// Custom update function that receives keyboard state each simulation tick.
pub type CustomInputUpdateFn =
    Box<dyn FnMut(f64, &nebula_input::KeyboardState, &nebula_input::MouseState)>;

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
    /// Optional custom update with keyboard state, called each simulation tick.
    pub custom_input_update: Option<CustomInputUpdateFn>,
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
    /// Six-face planet renderer.
    pub planet_faces: Option<PlanetFaces>,
    /// Planet mesh buffer.
    pub planet_face_mesh: Option<MeshBuffer>,
    /// Camera buffer for the planet face view.
    pub planet_camera_buffer: Option<wgpu::Buffer>,
    /// Camera bind group for the planet face view.
    pub planet_camera_bind_group: Option<wgpu::BindGroup>,
    /// No-cull lit pipeline for planet terrain with directional light shading.
    pub planet_pipeline: Option<LitPipeline>,
    /// Directional light (sun) for planet terrain shading.
    pub sun_light: DirectionalLight,
    /// GPU buffer for the directional light uniform.
    pub light_buffer: Option<wgpu::Buffer>,
    /// Bind group for the directional light uniform.
    pub light_bind_group: Option<wgpu::BindGroup>,
    /// Point light manager for local light sources.
    pub point_light_manager: PointLightManager,
    /// GPU storage buffer for point lights.
    pub point_light_buffer: Option<wgpu::Buffer>,
    /// Atmosphere scattering renderer.
    pub atmosphere_renderer: Option<AtmosphereRenderer>,
    /// Atmosphere bind group (recreated on depth buffer resize).
    pub atmosphere_bind_group: Option<wgpu::BindGroup>,
    /// Day/night cycle state (20-minute default cycle).
    pub day_night: DayNightState,
    /// Orbital planet renderer (textured sphere for orbit view).
    pub orbital_renderer: Option<OrbitalRenderer>,
    /// Ocean surface renderer.
    pub ocean_renderer: Option<OceanRenderer>,
    /// Rotation angle for the orbital planet (radians).
    pub orbital_rotation: f32,
    /// Surface-to-orbit transition configuration.
    pub transition_config: TransitionConfig,
    /// Coordinate origin manager for f32 precision.
    pub origin_manager: OriginManager,
    /// Current surface-to-orbit blend factor (0 = surface, 1 = orbital).
    pub transition_blend: f32,
    /// Current chunk budget based on altitude.
    pub chunk_budget: u32,
    /// Simulated camera altitude above planet surface (meters).
    pub simulated_altitude: f64,
    /// Planet impostor renderer (billboard for extreme distances).
    pub impostor_renderer: Option<ImpostorRenderer>,
    /// Impostor state tracking (view/sun direction for re-rendering).
    pub impostor_state: ImpostorState,
    /// Impostor configuration.
    pub impostor_config: ImpostorConfig,
    /// Procedural starfield skybox renderer.
    pub skybox_renderer: Option<SkyboxRenderer>,
    /// Bloom post-processing pipeline for HDR star rendering.
    pub bloom_pipeline: Option<BloomPipeline>,
    /// Sun corona renderer (billboard with animated HDR corona).
    pub sun_renderer: Option<SunRenderer>,
    /// Screen-space lens flare renderer.
    pub lens_flare: Option<nebula_render::LensFlareRenderer>,
    /// Distant planet impostor renderer (crescent-shaded billboards).
    pub distant_impostor: Option<PlanetImpostorRenderer>,
    /// Distant planets with orbital elements for impostor rendering.
    pub distant_planets: Vec<(DistantPlanet, OrbitalElements)>,
    /// Cascaded shadow map resources.
    pub shadow_maps: Option<CascadedShadowMaps>,
    /// Shadow depth-only render pipeline.
    pub shadow_pipeline: Option<ShadowPipeline>,
    /// GPU buffer for shadow uniform data.
    pub shadow_uniform_buffer: Option<wgpu::Buffer>,
    /// Per-cascade light matrix uniform buffers for shadow rendering.
    pub shadow_cascade_buffers: Vec<wgpu::Buffer>,
    /// Per-cascade bind groups for the shadow depth pass.
    pub shadow_cascade_bind_groups: Vec<wgpu::BindGroup>,
    /// Shadow bind group for the lit pipeline (group 2).
    pub shadow_bind_group: Option<wgpu::BindGroup>,
    /// PBR material for planet terrain.
    pub pbr_material: nebula_lighting::PbrMaterial,
    /// GPU buffer for PBR material uniform.
    pub material_buffer: Option<wgpu::Buffer>,
    /// Bind group for PBR material uniform (group 3).
    pub material_bind_group: Option<wgpu::BindGroup>,
    /// Space vs surface lighting context.
    pub lighting_context: LightingContext,
    /// Atmosphere config for altitude-based lighting transition.
    pub lighting_atmo_config: LightingAtmosphereConfig,
    /// GPU buffer for lighting context uniform.
    pub lighting_context_buffer: Option<wgpu::Buffer>,
    /// Frame-coherent keyboard state.
    pub keyboard_state: nebula_input::KeyboardState,
    /// Frame-coherent mouse state.
    pub mouse_state: nebula_input::MouseState,
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
            custom_input_update: None,
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
            planet_faces: None,
            planet_face_mesh: None,
            planet_camera_buffer: None,
            planet_camera_bind_group: None,
            planet_pipeline: None,
            sun_light: DirectionalLight::default(),
            light_buffer: None,
            light_bind_group: None,
            point_light_manager: PointLightManager::new(),
            point_light_buffer: None,
            atmosphere_renderer: None,
            atmosphere_bind_group: None,
            day_night: DayNightState::new(1200.0), // 20 minutes per day
            ocean_renderer: None,
            orbital_renderer: None,
            orbital_rotation: 0.0,
            transition_config: TransitionConfig::default(),
            origin_manager: OriginManager::new(),
            transition_blend: 0.0,
            chunk_budget: 4096,
            simulated_altitude: 0.0,
            impostor_renderer: None,
            impostor_state: ImpostorState::new(0.05),
            impostor_config: ImpostorConfig::default(),
            skybox_renderer: None,
            bloom_pipeline: None,
            sun_renderer: None,
            lens_flare: None,
            distant_impostor: None,
            distant_planets: Vec::new(),
            shadow_maps: None,
            shadow_pipeline: None,
            shadow_uniform_buffer: None,
            shadow_cascade_buffers: Vec::new(),
            shadow_cascade_bind_groups: Vec::new(),
            shadow_bind_group: None,
            pbr_material: nebula_lighting::PbrMaterial::stone(),
            material_buffer: None,
            material_bind_group: None,
            lighting_context: LightingContext::earth_like_surface(),
            lighting_atmo_config: LightingAtmosphereConfig::default(),
            lighting_context_buffer: None,
            keyboard_state: nebula_input::KeyboardState::new(),
            mouse_state: nebula_input::MouseState::new(),
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
            custom_input_update: None,
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
            planet_faces: None,
            planet_face_mesh: None,
            planet_camera_buffer: None,
            planet_camera_bind_group: None,
            planet_pipeline: None,
            sun_light: DirectionalLight::default(),
            light_buffer: None,
            light_bind_group: None,
            point_light_manager: PointLightManager::new(),
            point_light_buffer: None,
            atmosphere_renderer: None,
            atmosphere_bind_group: None,
            day_night: DayNightState::new(1200.0), // 20 minutes per day
            ocean_renderer: None,
            orbital_renderer: None,
            orbital_rotation: 0.0,
            transition_config: TransitionConfig::default(),
            origin_manager: OriginManager::new(),
            transition_blend: 0.0,
            chunk_budget: 4096,
            simulated_altitude: 0.0,
            impostor_renderer: None,
            impostor_state: ImpostorState::new(0.05),
            impostor_config: ImpostorConfig::default(),
            skybox_renderer: None,
            bloom_pipeline: None,
            sun_renderer: None,
            lens_flare: None,
            distant_impostor: None,
            distant_planets: Vec::new(),
            shadow_maps: None,
            shadow_pipeline: None,
            shadow_uniform_buffer: None,
            shadow_cascade_buffers: Vec::new(),
            shadow_cascade_bind_groups: Vec::new(),
            shadow_bind_group: None,
            pbr_material: nebula_lighting::PbrMaterial::stone(),
            material_buffer: None,
            material_bind_group: None,
            lighting_context: LightingContext::earth_like_surface(),
            lighting_atmo_config: LightingAtmosphereConfig::default(),
            lighting_context_buffer: None,
            keyboard_state: nebula_input::KeyboardState::new(),
            mouse_state: nebula_input::MouseState::new(),
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

        // --- Single face planet terrain (before moving unlit_pipeline) ---
        self.initialize_planet_face(gpu, &allocator, &unlit_pipeline);

        // --- Atmosphere scattering renderer ---
        let planet_radius = self
            .planet_faces
            .as_ref()
            .map(|p| p.planet_radius as f32)
            .unwrap_or(200.0);
        let atmo_params = AtmosphereParams::earth_like(planet_radius);
        let atmo_renderer = AtmosphereRenderer::new(&gpu.device, gpu.surface_format, atmo_params);
        let atmo_bind_group = atmo_renderer.create_bind_group(&gpu.device, &depth_buffer.view);
        self.atmosphere_renderer = Some(atmo_renderer);
        self.atmosphere_bind_group = Some(atmo_bind_group);

        // --- Orbital planet renderer ---
        self.initialize_orbital_renderer(gpu, planet_radius);

        // --- Planet impostor renderer (billboard for extreme distances) ---
        let impostor = ImpostorRenderer::new(
            &gpu.device,
            &gpu.queue,
            gpu.surface_format,
            self.impostor_config.texture_resolution,
        );
        info!(
            "Impostor renderer initialized: {}x{} texture",
            self.impostor_config.texture_resolution, self.impostor_config.texture_resolution
        );
        self.impostor_renderer = Some(impostor);

        // --- Bloom post-processing pipeline ---
        let hdr_format = wgpu::TextureFormat::Rgba16Float;
        let bloom = BloomPipeline::new(
            &gpu.device,
            hdr_format,
            gpu.surface_format,
            self.surface_width(),
            self.surface_height(),
            BloomConfig::default(),
        );
        self.bloom_pipeline = Some(bloom);

        // --- Procedural starfield skybox (renders to HDR target) ---
        let starfield_gen = StarfieldGenerator::new(42, 8000);
        let stars = starfield_gen.generate();
        let mut starfield_cubemap = StarfieldCubemap::render(&stars, 512);

        // Apply procedural nebula clouds onto the starfield cubemap.
        let nebula = NebulaGenerator::new(NebulaConfig {
            seed: 42,
            ..NebulaConfig::default()
        });
        starfield_cubemap.apply_nebula(&nebula);
        let skybox = SkyboxRenderer::new(
            &gpu.device,
            &gpu.queue,
            hdr_format, // HDR target format for bloom
            &starfield_cubemap,
        );
        self.skybox_renderer = Some(skybox);

        // --- Sun corona renderer (billboard in HDR space) ---
        let sun = SunRenderer::new(&gpu.device, hdr_format);
        self.sun_renderer = Some(sun);

        // --- Lens flare renderer (screen-space flare elements in HDR) ---
        let lens_flare = nebula_render::LensFlareRenderer::new(&gpu.device, hdr_format);
        self.lens_flare = Some(lens_flare);

        // --- Distant planet impostor renderer (crescent-shaded billboards) ---
        let distant_impostor = PlanetImpostorRenderer::new(&gpu.device, hdr_format);
        self.distant_impostor = Some(distant_impostor);

        // Demo distant planets: a Mars-like and a gas giant
        self.distant_planets = vec![
            (
                DistantPlanet {
                    id: 1,
                    position: [0; 3],
                    radius: planet_radius as f64 * 0.6,
                    albedo: 0.25,
                    color: [0.9, 0.5, 0.3],
                    has_atmosphere: false,
                    atmosphere_color: [0.0; 3],
                },
                OrbitalElements {
                    semi_major_axis: planet_radius as f64 * 12.0,
                    eccentricity: 0.05,
                    inclination: 0.05,
                    longitude_ascending: 0.0,
                    argument_periapsis: 0.0,
                    mean_anomaly_epoch: 0.0,
                    orbital_period: 200.0,
                },
            ),
            (
                DistantPlanet {
                    id: 2,
                    position: [0; 3],
                    radius: planet_radius as f64 * 1.5,
                    albedo: 0.4,
                    color: [0.7, 0.6, 0.4],
                    has_atmosphere: true,
                    atmosphere_color: [0.5, 0.7, 1.0],
                },
                OrbitalElements {
                    semi_major_axis: planet_radius as f64 * 20.0,
                    eccentricity: 0.02,
                    inclination: 0.1,
                    longitude_ascending: 1.0,
                    argument_periapsis: 0.5,
                    mean_anomaly_epoch: 1.5,
                    orbital_period: 400.0,
                },
            ),
            (
                DistantPlanet {
                    id: 3,
                    position: [0; 3],
                    radius: planet_radius as f64 * 0.4,
                    albedo: 0.6,
                    color: [0.9, 0.9, 0.8],
                    has_atmosphere: true,
                    atmosphere_color: [0.8, 0.85, 1.0],
                },
                OrbitalElements {
                    semi_major_axis: planet_radius as f64 * 6.0,
                    eccentricity: 0.0,
                    inclination: 0.02,
                    longitude_ascending: 0.5,
                    argument_periapsis: 0.0,
                    mean_anomaly_epoch: 2.5,
                    orbital_period: 80.0,
                },
            ),
        ];
        info!(
            "Distant planet impostors initialized: {} planets",
            self.distant_planets.len()
        );

        // --- Ocean surface renderer ---
        self.initialize_ocean_renderer(gpu, planet_radius);

        self.unlit_pipeline = Some(unlit_pipeline);
        self.triangle_mesh = Some(triangle_mesh);
        self.back_triangle_mesh = Some(back_triangle_mesh);
        self.depth_buffer = Some(depth_buffer);
        self.camera_buffer = Some(camera_buffer);
        self.camera_bind_group = Some(camera_bind_group);

        // Create cube-face quads using CubeFace basis vectors
        self.cube_face_meshes = create_cube_face_meshes(&allocator);

        info!(
            "Rendering pipeline initialized successfully with depth buffer, textured quad, cube faces, and planet face"
        );
    }

    /// Initialize the orbital planet renderer (textured icosphere for orbit view).
    fn initialize_orbital_renderer(&mut self, gpu: &RenderContext, planet_radius: f32) {
        use nebula_planet::orbital::texture::create_default_samplers;

        let mesh = generate_orbital_sphere(5);
        let (terrain, biome) = create_default_samplers(42, planet_radius as f64);
        let tex_width = 512;
        let tex_height = 256;
        let terrain_pixels =
            nebula_planet::generate_terrain_color_texture(&terrain, &biome, tex_width, tex_height);

        let orbital = OrbitalRenderer::new(
            &gpu.device,
            &gpu.queue,
            gpu.surface_format,
            &mesh,
            &terrain_pixels,
            tex_width,
            tex_height,
            planet_radius,
        );

        info!(
            "Orbital renderer initialized: {} vertices, {} triangles, {}x{} texture",
            mesh.positions.len(),
            mesh.indices.len() / 3,
            tex_width,
            tex_height
        );
        self.orbital_renderer = Some(orbital);
    }

    /// Initialize the ocean surface renderer.
    fn initialize_ocean_renderer(&mut self, gpu: &RenderContext, planet_radius: f32) {
        let mesh = generate_orbital_sphere(4); // slightly lower res than orbital
        let params = OceanParams::default();
        let ocean = OceanRenderer::new(
            &gpu.device,
            gpu.surface_format,
            &mesh,
            params,
            planet_radius,
        );
        info!(
            "Ocean renderer initialized: {} vertices, {} triangles",
            mesh.positions.len(),
            mesh.indices.len() / 3
        );
        self.ocean_renderer = Some(ocean);
    }

    /// Initialize six-face planet terrain rendering with directional light shading.
    fn initialize_planet_face(
        &mut self,
        gpu: &RenderContext,
        allocator: &BufferAllocator,
        _unlit_pipeline: &UnlitPipeline,
    ) {
        use wgpu::util::DeviceExt;
        let mut shader_library = ShaderLibrary::new();
        let shader = shader_library
            .load_from_source(&gpu.device, "planet-lit", LIT_SHADER_SOURCE)
            .expect("Failed to load planet lit shader");
        let planet_pipeline = LitPipeline::new(
            &gpu.device,
            &shader,
            gpu.surface_format,
            Some(DepthBuffer::FORMAT),
            None, // No culling for cubesphere terrain
        );

        // Create directional light uniform buffer and bind group.
        let light_uniform = self.sun_light.to_uniform();
        let light_buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("directional-light-uniform"),
                contents: bytemuck::cast_slice(&[light_uniform]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        // Create point light storage buffer (header + max lights).
        let point_light_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point-light-storage"),
            size: PointLightManager::BUFFER_SIZE,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Initialize header to zero lights.
        let zero_header = nebula_lighting::PointLightHeader {
            count: 0,
            _pad: [0; 3],
        };
        gpu.queue
            .write_buffer(&point_light_buffer, 0, bytemuck::cast_slice(&[zero_header]));

        // Create lighting context uniform buffer.
        let lighting_ctx_uniform = self.lighting_context.to_uniform();
        let lighting_context_buffer =
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("lighting-context-uniform"),
                    contents: bytemuck::cast_slice(&[lighting_ctx_uniform]),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });

        let light_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("directional-light-bind-group"),
            layout: &planet_pipeline.light_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: point_light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: lighting_context_buffer.as_entire_binding(),
                },
            ],
        });
        self.light_buffer = Some(light_buffer);
        self.light_bind_group = Some(light_bind_group);
        self.point_light_buffer = Some(point_light_buffer);
        self.lighting_context_buffer = Some(lighting_context_buffer);

        // --- Cascaded Shadow Maps ---
        self.initialize_shadow_maps(gpu, &mut shader_library, &planet_pipeline);

        // --- PBR Material Uniform ---
        self.initialize_pbr_material(gpu, &planet_pipeline);

        let planet = PlanetFaces::new_demo(1, 42);
        // Add demo point lights on the planet surface.
        self.initialize_demo_point_lights(planet.planet_radius as f32);
        let planet_radius = planet.planet_radius;
        let (vertices, indices) = planet.visible_render_data();
        if vertices.is_empty() {
            info!("Planet: no vertices across 6 faces");
            self.planet_faces = Some(planet);
            self.planet_pipeline = Some(planet_pipeline);
            return;
        }
        let mesh = allocator.create_mesh(
            "planet-six-faces",
            bytemuck::cast_slice(&vertices),
            IndexData::U32(&indices),
        );
        let aspect = self.surface_width() as f32 / self.surface_height().max(1) as f32;
        let vp = create_orbit_camera(planet_radius as f32, 200.0, 0.0, 0.6, aspect);
        let uniform = CameraUniform {
            view_proj: vp.to_cols_array_2d(),
            camera_pos: [0.0, 0.0, 0.0, 0.0],
        };
        let buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("planet-camera-uniform"),
                contents: bytemuck::cast_slice(&[uniform]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("planet-camera-bind-group"),
            layout: &planet_pipeline.camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        info!(
            "Planet six-face: {} verts, {} tris",
            vertices.len(),
            indices.len() / 3
        );
        self.planet_faces = Some(planet);
        self.planet_face_mesh = Some(mesh);
        self.planet_camera_buffer = Some(buffer);
        self.planet_camera_bind_group = Some(bind_group);
        self.planet_pipeline = Some(planet_pipeline);
    }

    /// Place demo point lights at intervals on the planet surface.
    fn initialize_demo_point_lights(&mut self, planet_radius: f32) {
        // Place 12 warm-orange point lights around the equator.
        let count = 12;
        for i in 0..count {
            let angle = (i as f32 / count as f32) * std::f32::consts::TAU;
            let x = angle.cos() * planet_radius;
            let z = angle.sin() * planet_radius;
            // Slightly above surface.
            let dir = glam::Vec3::new(x, 0.0, z).normalize();
            let pos = dir * (planet_radius + 1.0);
            self.point_light_manager.add(PointLight {
                position: pos,
                color: glam::Vec3::new(1.0, 0.7, 0.3), // warm orange
                intensity: 3.0,
                radius: 30.0,
            });
        }
        info!(
            "Demo point lights: {} placed on planet surface",
            self.point_light_manager.len()
        );
    }

    /// Initialize PBR material uniform buffer and bind group.
    fn initialize_pbr_material(&mut self, gpu: &RenderContext, planet_pipeline: &LitPipeline) {
        use wgpu::util::DeviceExt;

        let mat_uniform = self.pbr_material.to_uniform();
        let material_buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("pbr-material-uniform"),
                contents: bytemuck::cast_slice(&[mat_uniform]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let material_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pbr-material-bind-group"),
            layout: &planet_pipeline.material_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: material_buffer.as_entire_binding(),
            }],
        });
        self.material_buffer = Some(material_buffer);
        self.material_bind_group = Some(material_bind_group);
    }

    /// Initialize cascaded shadow map resources.
    fn initialize_shadow_maps(
        &mut self,
        gpu: &RenderContext,
        shader_library: &mut ShaderLibrary,
        planet_pipeline: &LitPipeline,
    ) {
        use wgpu::util::DeviceExt;

        let config = CascadedShadowConfig::default();
        let shadow_maps = CascadedShadowMaps::new(&gpu.device, &config);

        // Shadow depth-only pipeline
        let shadow_shader = shader_library
            .load_from_source(&gpu.device, "shadow-depth", SHADOW_SHADER_SOURCE)
            .expect("Failed to load shadow shader");
        let shadow_pipe = ShadowPipeline::new(&gpu.device, &shadow_shader);

        // Per-cascade light matrix buffers and bind groups
        let mut cascade_buffers = Vec::with_capacity(config.cascade_count as usize);
        let mut cascade_bind_groups = Vec::with_capacity(config.cascade_count as usize);
        for i in 0..config.cascade_count as usize {
            let buf = gpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("shadow-cascade-{i}-matrix")),
                    contents: bytemuck::cast_slice(&shadow_maps.light_matrices[i].to_cols_array()),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });
            let bg = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("shadow-cascade-{i}-bg")),
                layout: &shadow_pipe.light_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buf.as_entire_binding(),
                }],
            });
            cascade_buffers.push(buf);
            cascade_bind_groups.push(bg);
        }

        // Shadow uniform buffer (for fragment shader sampling)
        let shadow_uniform = shadow_maps.to_uniform();
        let shadow_uniform_buffer =
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("shadow-uniform"),
                    contents: bytemuck::cast_slice(&[shadow_uniform]),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });

        // Shadow bind group for the lit pipeline (group 2)
        let shadow_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow-bind-group"),
            layout: &planet_pipeline.shadow_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: shadow_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&shadow_maps.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&shadow_maps.sampler),
                },
            ],
        });

        info!(
            "Cascaded shadow maps initialized: {} cascades, {}x{} resolution",
            config.cascade_count, config.resolution, config.resolution
        );

        self.shadow_maps = Some(shadow_maps);
        self.shadow_pipeline = Some(shadow_pipe);
        self.shadow_uniform_buffer = Some(shadow_uniform_buffer);
        self.shadow_cascade_buffers = cascade_buffers;
        self.shadow_cascade_bind_groups = cascade_bind_groups;
        self.shadow_bind_group = Some(shadow_bind_group);
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

        // Compute planetary coordinates from camera position.
        let planetary_position = self.compute_planetary_position();

        if let Ok(mut state) = self.debug_state.lock() {
            state.frame_count = self.game_loop.frame_count();
            state.frame_time_ms = frame_time_ms;
            state.fps = fps;
            state.entity_count = 0; // Will be updated once ECS is implemented
            state.window_width = self.surface_width();
            state.window_height = self.surface_height();
            state.uptime_seconds = uptime_seconds;
            state.planetary_position = planetary_position;
        }

        self.last_frame_time = now;
    }

    /// Compute the camera's planetary coordinate string for the debug HUD.
    ///
    /// Uses the demo planet (centered at origin) and the camera's f32 position.
    /// Returns an empty string if no planet is loaded.
    fn compute_planetary_position(&self) -> String {
        let planet_radius = match &self.planet_faces {
            Some(pf) => pf.planet_radius,
            None => return String::new(),
        };

        let cam = self.camera.position;
        let dx = cam.x as f64;
        let dy = cam.y as f64;
        let dz = cam.z as f64;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();

        if dist < 1e-10 {
            return String::from("0.0°N, 0.0°E, 0m alt");
        }

        let dir_y = dy / dist;
        let latitude = dir_y.asin().to_degrees();
        let longitude = dz.atan2(dx).to_degrees();
        let altitude = dist - planet_radius;

        let coord = PlanetaryCoord {
            latitude,
            longitude,
            altitude,
        };
        format!("{coord}")
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
                        // Recreate atmosphere bind group with new depth view
                        if let Some(atmo) = &self.atmosphere_renderer {
                            self.atmosphere_bind_group =
                                Some(atmo.create_bind_group(&gpu.device, &depth_buffer.view));
                        }
                    }

                    // Resize bloom pipeline
                    if let Some(gpu) = &self.gpu
                        && let Some(bloom) = &mut self.bloom_pipeline
                    {
                        bloom.resize(&gpu.device, w, h);
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
                            if let Some(atmo) = &self.atmosphere_renderer {
                                self.atmosphere_bind_group =
                                    Some(atmo.create_bind_group(&gpu.device, &depth_buffer.view));
                            }
                        }

                        if let Some(gpu) = &self.gpu
                            && let Some(bloom) = &mut self.bloom_pipeline
                        {
                            bloom.resize(&gpu.device, w, h);
                        }

                        info!(
                            "Scale factor changed to {:.2}, resized to {}x{}",
                            scale_factor, w, h
                        );
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.keyboard_state.process_event(&event);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_state.on_cursor_moved(position.x, position.y);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.mouse_state.on_button(button, state);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.mouse_state.on_scroll(delta);
            }
            WindowEvent::CursorEntered { .. } => {
                self.mouse_state.on_cursor_entered();
            }
            WindowEvent::CursorLeft { .. } => {
                self.mouse_state.on_cursor_left();
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
                let custom_input_update = &mut self.custom_input_update;
                let keyboard_state = &self.keyboard_state;
                let mouse_state = &self.mouse_state;
                let camera = &mut self.camera;
                let camera_time = &mut self.camera_time;
                let camera_buffer = &self.camera_buffer;
                let gpu = &self.gpu;
                let day_night = &mut self.day_night;
                let orbital_rotation = &mut self.orbital_rotation;
                let transition_config = &self.transition_config;
                let transition_blend = &mut self.transition_blend;
                let chunk_budget = &mut self.chunk_budget;
                let simulated_altitude = &mut self.simulated_altitude;

                self.game_loop.tick(
                    |dt, _sim_time| {
                        *tick_count += 1;
                        *camera_time += dt;
                        day_night.tick(dt);
                        // Slow planet rotation (~1 revolution per 10 minutes)
                        *orbital_rotation += (dt as f32) * 0.01;

                        // Simulate altitude oscillation: 0 → 300 km → 0 over ~60s
                        let alt_cycle = (*camera_time * 0.1).sin().abs();
                        *simulated_altitude = alt_cycle * 300_000.0;

                        // Update transition blend and chunk budget
                        let (_mode, blend) = transition_config.classify(*simulated_altitude);
                        *transition_blend = blend;
                        *chunk_budget =
                            chunk_budget_for_altitude(*simulated_altitude, transition_config);

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
                        if let Some(update_fn) = custom_input_update {
                            update_fn(dt, keyboard_state, mouse_state);
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

                            // Log transition state periodically
                            if self.tick_count.is_multiple_of(120) {
                                let (mode, _) =
                                    self.transition_config.classify(self.simulated_altitude);
                                info!(
                                    "Transition: alt={:.0}km, blend={:.2}, budget={}, mode={:?}",
                                    self.simulated_altitude / 1000.0,
                                    self.transition_blend,
                                    self.chunk_budget,
                                    mode,
                                );
                            }

                            // Determine which passes to run based on blend
                            let render_orbital = self.transition_blend > 0.0;
                            let render_voxels = self.transition_blend < 1.0;

                            // === Pass -1: Starfield skybox → HDR + Bloom ===
                            if let (Some(skybox), Some(bloom)) =
                                (&self.skybox_renderer, &self.bloom_pipeline)
                            {
                                // Build rotation-only inverse VP for skybox
                                let aspect = self.surface_width() as f32
                                    / self.surface_height().max(1) as f32;
                                let orbit_angle = self.camera_time * 0.3;
                                let planet_radius = self
                                    .planet_faces
                                    .as_ref()
                                    .map(|p| p.planet_radius as f32)
                                    .unwrap_or(200.0);
                                let altitude = planet_radius * 3.0;
                                let dist = planet_radius + altitude;
                                let tilt = 0.4_f32;
                                let eye = glam::Vec3::new(
                                    (orbit_angle.sin() as f32) * dist * tilt.cos(),
                                    dist * tilt.sin(),
                                    (orbit_angle.cos() as f32) * dist * tilt.cos(),
                                );
                                let forward = (-eye).normalize();
                                let right = forward.cross(glam::Vec3::Y).normalize();
                                let up = right.cross(forward).normalize();
                                let view_rot = glam::Mat4::from_cols(
                                    glam::Vec4::new(right.x, up.x, -forward.x, 0.0),
                                    glam::Vec4::new(right.y, up.y, -forward.y, 0.0),
                                    glam::Vec4::new(right.z, up.z, -forward.z, 0.0),
                                    glam::Vec4::new(0.0, 0.0, 0.0, 1.0),
                                );
                                let proj = glam::Mat4::perspective_rh(
                                    70.0_f32.to_radians(),
                                    aspect,
                                    0.1,
                                    100.0,
                                );
                                let vp_rot = proj * view_rot;
                                let inv_vp = vp_rot.inverse();
                                skybox.update(&gpu.queue, inv_vp);

                                // Render skybox to HDR texture
                                let hdr_clear = wgpu::Color {
                                    r: clear_color.r * 0.1,
                                    g: clear_color.g * 0.1,
                                    b: clear_color.b * 0.1,
                                    a: 1.0,
                                };
                                let pb = RenderPassBuilder::new()
                                    .clear_color(hdr_clear)
                                    .label("skybox-hdr-pass");
                                {
                                    let mut pass =
                                        frame_encoder.begin_render_pass_to(&pb, bloom.hdr_view());
                                    skybox.render(&mut pass);
                                }

                                // Render sun corona to HDR (additive, after skybox)
                                if let Some(sun_renderer) = &self.sun_renderer {
                                    let sun_dir = self.day_night.sun_direction;
                                    let sun_props = SunProperties {
                                        direction: sun_dir,
                                        physical_diameter: 1_392_700.0,
                                        distance: 149_597_870.0,
                                        star_type: StarType::G,
                                        luminosity: 1.0,
                                    };

                                    // Build a Camera struct matching the skybox view
                                    let cam_forward = forward;
                                    let cam_right = right;
                                    let cam_up = up;
                                    let rot_mat =
                                        glam::Mat3::from_cols(cam_right, cam_up, -cam_forward);
                                    let cam_rotation = glam::Quat::from_mat3(&rot_mat);
                                    let sky_camera = Camera {
                                        position: glam::Vec3::ZERO,
                                        rotation: cam_rotation,
                                        ..Camera::default()
                                    };

                                    sun_renderer.update(
                                        &gpu.queue,
                                        vp_rot,
                                        &sky_camera,
                                        &sun_props,
                                        self.camera_time as f32,
                                    );

                                    let sun_pb = RenderPassBuilder::new()
                                        .preserve_color()
                                        .label("sun-corona-hdr-pass");
                                    {
                                        let mut sun_pass = frame_encoder
                                            .begin_render_pass_to(&sun_pb, bloom.hdr_view());
                                        sun_renderer.render(&mut sun_pass);
                                    }
                                }

                                // Render lens flare to HDR (additive, after sun)
                                if let Some(flare) = &self.lens_flare {
                                    let sun_dir = self.day_night.sun_direction;
                                    let sun_brightness = 50.0_f32; // matches SunProperties::hdr_brightness
                                    let visible =
                                        flare.update(&gpu.queue, sun_dir, sun_brightness, vp_rot);
                                    if visible {
                                        let flare_pb = RenderPassBuilder::new()
                                            .preserve_color()
                                            .label("lens-flare-hdr-pass");
                                        let mut flare_pass = frame_encoder
                                            .begin_render_pass_to(&flare_pb, bloom.hdr_view());
                                        flare.render(&mut flare_pass, flare.element_count());
                                    }
                                }

                                // Render distant planet impostors to HDR (after sun/flare, before bloom)
                                if let Some(distant) = &mut self.distant_impostor {
                                    let time_s = self.camera_time;
                                    let sun_dir = self.day_night.sun_direction;

                                    // Build a Camera struct for billboard orientation
                                    let rot_mat = glam::Mat3::from_cols(right, up, -forward);
                                    let cam_rotation = glam::Quat::from_mat3(&rot_mat);
                                    let sky_cam = Camera {
                                        position: glam::Vec3::ZERO,
                                        rotation: cam_rotation,
                                        ..Camera::default()
                                    };

                                    let mut instances = Vec::new();
                                    for (planet, orbit) in &self.distant_planets {
                                        let orbital_pos = orbit.position_at_time(time_s);
                                        let rel = orbital_pos.as_vec3();
                                        let dist = rel.length() as f64;
                                        if dist < 1e-6 {
                                            continue;
                                        }
                                        let direction = rel / rel.length();
                                        let ang_radius =
                                            (planet.angular_diameter(dist) * 0.5) as f32;
                                        // Ensure minimum visible size
                                        let ang_radius = ang_radius.max(0.003);

                                        let planet_to_sun = glam::DVec3::from(sun_dir).normalize();
                                        let to_observer = -orbital_pos.normalize();
                                        let phase_angle = DistantPlanet::compute_phase_angle(
                                            planet_to_sun,
                                            to_observer,
                                        );
                                        let brightness = planet.phase_brightness(phase_angle);

                                        let sun_local =
                                            billboard_local_sun_dir(direction, sun_dir, &sky_cam);

                                        instances.push(ImpostorInstance {
                                            position: direction.into(),
                                            scale: ang_radius,
                                            color: planet.color,
                                            brightness: brightness.max(0.15),
                                            sun_dir_local: sun_local.into(),
                                            has_atmosphere: planet.has_atmosphere as u32,
                                            atmosphere_color: planet.atmosphere_color,
                                            _padding: 0.0,
                                        });
                                    }

                                    distant.update(&gpu.queue, vp_rot, &instances);

                                    let impostor_pb = RenderPassBuilder::new()
                                        .preserve_color()
                                        .label("distant-planet-impostor-pass");
                                    {
                                        let mut pass = frame_encoder
                                            .begin_render_pass_to(&impostor_pb, bloom.hdr_view());
                                        distant.render(&mut pass);
                                    }
                                }

                                // Run bloom: extract → blur → tonemap → composite
                                let (encoder, surface_view) = frame_encoder.encoder_and_view();
                                bloom.execute(encoder, surface_view);
                            }

                            // === Pass 0: Orbital planet sphere (or clear-only) ===
                            if let (Some(orbital), Some(depth_buffer)) =
                                (&self.orbital_renderer, &self.depth_buffer)
                            {
                                let aspect = self.surface_width() as f32
                                    / self.surface_height().max(1) as f32;
                                let orbit_angle = self.camera_time * 0.3;
                                let planet_radius = self
                                    .planet_faces
                                    .as_ref()
                                    .map(|p| p.planet_radius as f32)
                                    .unwrap_or(200.0);
                                let altitude = planet_radius * 3.0;
                                let vp = create_orbit_camera(
                                    planet_radius,
                                    altitude,
                                    orbit_angle,
                                    0.4,
                                    aspect,
                                );

                                orbital.update(
                                    &gpu.queue,
                                    vp,
                                    glam::Vec3::ZERO,
                                    self.day_night.sun_direction,
                                    self.orbital_rotation,
                                );

                                let pb = if self.skybox_renderer.is_some() {
                                    RenderPassBuilder::new()
                                        .preserve_color()
                                        .depth(depth_buffer.view.clone(), DepthBuffer::CLEAR_VALUE)
                                        .label("orbital-planet-pass")
                                } else {
                                    RenderPassBuilder::new()
                                        .clear_color(clear_color)
                                        .depth(depth_buffer.view.clone(), DepthBuffer::CLEAR_VALUE)
                                        .label("orbital-planet-pass")
                                };
                                {
                                    let mut pass = frame_encoder.begin_render_pass(&pb);
                                    // Only draw orbital sphere when blend > 0
                                    if render_orbital {
                                        orbital.render(&mut pass);
                                    }
                                }
                            }

                            // === Pass 0.5: Impostor billboard for a distant planet ===
                            if let (Some(impostor), Some(depth_buffer)) =
                                (&self.impostor_renderer, &self.depth_buffer)
                            {
                                let aspect = self.surface_width() as f32
                                    / self.surface_height().max(1) as f32;
                                let orbit_angle = self.camera_time * 0.3;
                                let planet_radius = self
                                    .planet_faces
                                    .as_ref()
                                    .map(|p| p.planet_radius as f32)
                                    .unwrap_or(200.0);
                                let altitude = planet_radius * 3.0;
                                let vp = create_orbit_camera(
                                    planet_radius,
                                    altitude,
                                    orbit_angle,
                                    0.4,
                                    aspect,
                                );

                                // Place a distant "second planet" off to the side
                                let distant_center = glam::Vec3::new(planet_radius * 8.0, 0.0, 0.0);
                                let cam_dist_f64 = planet_radius as f64 + altitude as f64;
                                let cam_pos = glam::Vec3::new(
                                    (orbit_angle.cos() * 0.4_f64.cos() * cam_dist_f64) as f32,
                                    (0.4_f64.sin() * cam_dist_f64) as f32,
                                    (orbit_angle.sin() * 0.4_f64.cos() * cam_dist_f64) as f32,
                                );

                                // Camera basis vectors for billboard orientation
                                let to_planet = (distant_center - cam_pos).normalize();
                                let cam_right = to_planet.cross(glam::Vec3::Y).normalize();
                                let cam_up = cam_right.cross(to_planet).normalize();

                                let dist_to_planet = (distant_center - cam_pos).length() as f64;
                                let half_size =
                                    impostor_quad_size(planet_radius as f64 * 0.5, dist_to_planet)
                                        / 2.0;

                                impostor.update(
                                    &gpu.queue,
                                    vp,
                                    distant_center,
                                    cam_right,
                                    cam_up,
                                    half_size,
                                );

                                let pb = RenderPassBuilder::new()
                                    .preserve_color()
                                    .depth(depth_buffer.view.clone(), DepthBuffer::CLEAR_VALUE)
                                    .preserve_depth()
                                    .label("impostor-pass");
                                {
                                    let mut pass = frame_encoder.begin_render_pass(&pb);
                                    impostor.render(&mut pass);
                                }
                            }

                            // === Pass 1: Six-face planet with directional light ===
                            if render_voxels
                                && let (
                                    Some(pipeline),
                                    Some(cam_bg),
                                    Some(light_bg),
                                    Some(depth_buffer),
                                ) = (
                                    &self.planet_pipeline,
                                    &self.planet_camera_bind_group,
                                    &self.light_bind_group,
                                    &self.depth_buffer,
                                )
                            {
                                if let Some(planet_buf) = &self.planet_camera_buffer {
                                    let aspect = self.surface_width() as f32
                                        / self.surface_height().max(1) as f32;
                                    let orbit_angle = self.camera_time * 0.3;
                                    let planet_radius = self
                                        .planet_faces
                                        .as_ref()
                                        .map(|p| p.planet_radius as f32)
                                        .unwrap_or(200.0);
                                    let altitude = planet_radius * 0.8;
                                    let vp = create_orbit_camera(
                                        planet_radius,
                                        altitude,
                                        orbit_angle,
                                        0.4,
                                        aspect,
                                    );

                                    // Level 1 (coarse): face-level frustum culling
                                    let frustum = nebula_render::Frustum::from_view_projection(&vp);
                                    // Level 2 (fine): chunk-level culling via LocalFrustum
                                    let _local_frustum = LocalFrustum::from_view_proj(&vp);
                                    if let Some(planet) = &mut self.planet_faces {
                                        let visible_faces = planet.cull_faces(&frustum);

                                        // Gather visible render data after face culling
                                        let (verts, idxs) = planet.visible_render_data();

                                        // Chunk-level culling stats (faces are chunks here)
                                        let total_faces = 6u32;
                                        let visible_pct = if total_faces > 0 {
                                            visible_faces as f32 / total_faces as f32 * 100.0
                                        } else {
                                            0.0
                                        };

                                        // Log culling stats periodically
                                        if self.tick_count.is_multiple_of(60) {
                                            info!(
                                                "Frustum culled: {:.0}% of chunks ({} visible / {} loaded)",
                                                100.0 - visible_pct,
                                                visible_faces,
                                                total_faces,
                                            );
                                        }

                                        if !verts.is_empty() {
                                            let alloc = BufferAllocator::new(&gpu.device);
                                            self.planet_face_mesh = Some(alloc.create_mesh(
                                                "planet-six-faces",
                                                bytemuck::cast_slice(&verts),
                                                IndexData::U32(&idxs),
                                            ));
                                        }
                                    }
                                    // Compute camera position for PBR view direction.
                                    let cam_dist_f64_planet =
                                        planet_radius as f64 + altitude as f64;
                                    let planet_cam_pos = glam::Vec3::new(
                                        (orbit_angle.cos() * 0.4_f64.cos() * cam_dist_f64_planet)
                                            as f32,
                                        (0.4_f64.sin() * cam_dist_f64_planet) as f32,
                                        (orbit_angle.sin() * 0.4_f64.cos() * cam_dist_f64_planet)
                                            as f32,
                                    );
                                    let uniform = CameraUniform {
                                        view_proj: vp.to_cols_array_2d(),
                                        camera_pos: [
                                            planet_cam_pos.x,
                                            planet_cam_pos.y,
                                            planet_cam_pos.z,
                                            0.0,
                                        ],
                                    };
                                    gpu.queue.write_buffer(
                                        planet_buf,
                                        0,
                                        bytemuck::cast_slice(&[uniform]),
                                    );

                                    // Update directional light from day/night cycle.
                                    self.sun_light.set_direction(self.day_night.sun_direction);
                                    self.sun_light.color = glam::Vec3::new(1.0, 0.96, 0.90);
                                    self.sun_light.intensity = self.day_night.sun_intensity;
                                    if let Some(lb) = &self.light_buffer {
                                        let light_u = self.sun_light.to_uniform();
                                        gpu.queue.write_buffer(
                                            lb,
                                            0,
                                            bytemuck::cast_slice(&[light_u]),
                                        );
                                    }

                                    // Update space vs surface lighting context.
                                    let surface_ctx = LightingContext::earth_like_surface();
                                    let mut ctx = lighting_context_at_altitude(
                                        self.simulated_altitude,
                                        &self.lighting_atmo_config,
                                        &surface_ctx,
                                    );
                                    // Modulate ambient by sun elevation (day/night).
                                    let sun_elev = self.day_night.sun_direction.y;
                                    ctx.ambient_color =
                                        modulate_ambient_by_sun(ctx.ambient_color, sun_elev);
                                    self.lighting_context = ctx.clone();
                                    if let Some(lcb) = &self.lighting_context_buffer {
                                        let lcu = ctx.to_uniform();
                                        gpu.queue.write_buffer(
                                            lcb,
                                            0,
                                            bytemuck::cast_slice(&[lcu]),
                                        );
                                    }

                                    // Upload point lights (cull against current frustum).
                                    let cam_dist_f64 = planet_radius as f64 + altitude as f64;
                                    let cam_pos = glam::Vec3::new(
                                        (orbit_angle.cos() * 0.4_f64.cos() * cam_dist_f64) as f32,
                                        (0.4_f64.sin() * cam_dist_f64) as f32,
                                        (orbit_angle.sin() * 0.4_f64.cos() * cam_dist_f64) as f32,
                                    );
                                    if let Some(pl_buf) = &self.point_light_buffer {
                                        let pl_frustum =
                                            PointLightFrustum::from_planes(frustum.planes());
                                        self.point_light_manager.cull_and_upload(
                                            cam_pos,
                                            &pl_frustum,
                                            &gpu.queue,
                                            pl_buf,
                                        );
                                    }

                                    // Update shadow cascade matrices.
                                    if let Some(shadow_maps) = &mut self.shadow_maps {
                                        let light_dir = self.sun_light.direction;
                                        let inv_vp = vp.inverse();
                                        shadow_maps.update_matrices(light_dir, inv_vp, 0.1);

                                        // Upload per-cascade light matrices.
                                        for (i, buf) in
                                            self.shadow_cascade_buffers.iter().enumerate()
                                        {
                                            gpu.queue.write_buffer(
                                                buf,
                                                0,
                                                bytemuck::cast_slice(
                                                    &shadow_maps.light_matrices[i].to_cols_array(),
                                                ),
                                            );
                                        }

                                        // Upload shadow uniform for fragment shader.
                                        if let Some(sub) = &self.shadow_uniform_buffer {
                                            let su = shadow_maps.to_uniform();
                                            gpu.queue.write_buffer(
                                                sub,
                                                0,
                                                bytemuck::cast_slice(&[su]),
                                            );
                                        }
                                    }
                                }

                                // Render shadow cascades (depth-only passes).
                                if let (Some(shadow_pipe), Some(planet_mesh), Some(shadow_maps)) = (
                                    &self.shadow_pipeline,
                                    &self.planet_face_mesh,
                                    &self.shadow_maps,
                                ) {
                                    let (encoder, _) = frame_encoder.encoder_and_view();
                                    render_shadow_cascades(
                                        encoder,
                                        shadow_pipe,
                                        &shadow_maps.cascade_views,
                                        &self.shadow_cascade_bind_groups,
                                        planet_mesh,
                                    );
                                }

                                if let (Some(planet_mesh), Some(shadow_bg), Some(mat_bg)) = (
                                    &self.planet_face_mesh,
                                    &self.shadow_bind_group,
                                    &self.material_bind_group,
                                ) {
                                    let pb = RenderPassBuilder::new()
                                        .preserve_color()
                                        .depth(depth_buffer.view.clone(), DepthBuffer::CLEAR_VALUE)
                                        .label("planet-six-face-pass");
                                    {
                                        let mut pass = frame_encoder.begin_render_pass(&pb);
                                        draw_lit(
                                            &mut pass,
                                            pipeline,
                                            cam_bg,
                                            light_bg,
                                            shadow_bg,
                                            mat_bg,
                                            planet_mesh,
                                        );
                                    }
                                }
                            }

                            // === Pass 1.25: Ocean surface (after terrain, before atmosphere) ===
                            {
                                let sw = self.surface_wrapper.physical_width();
                                let sh = self.surface_wrapper.physical_height();
                                let aspect = sw as f32 / sh.max(1) as f32;
                                let orbit_angle = self.camera_time * 0.3;
                                let planet_radius = self
                                    .planet_faces
                                    .as_ref()
                                    .map(|p| p.planet_radius as f32)
                                    .unwrap_or(200.0);
                                let altitude = planet_radius * 0.8;
                                let vp = create_orbit_camera(
                                    planet_radius,
                                    altitude,
                                    orbit_angle,
                                    0.4,
                                    aspect,
                                );
                                let cam_dist = planet_radius + altitude;
                                let cam_pos = glam::Vec3::new(
                                    (orbit_angle.cos() * 0.4_f64.cos() * cam_dist as f64) as f32,
                                    (0.4_f64.sin() * cam_dist as f64) as f32,
                                    (orbit_angle.sin() * 0.4_f64.cos() * cam_dist as f64) as f32,
                                );
                                let sun_dir = self.day_night.sun_direction;
                                let dt = crate::game_loop::FIXED_DT as f32;

                                if let Some(ocean) = &mut self.ocean_renderer {
                                    ocean.update(&gpu.queue, vp, sun_dir, cam_pos, dt);
                                }

                                if let (Some(ocean), Some(depth_buffer)) =
                                    (&self.ocean_renderer, &self.depth_buffer)
                                {
                                    let ocean_pass_builder = RenderPassBuilder::new()
                                        .preserve_color()
                                        .depth(depth_buffer.view.clone(), DepthBuffer::CLEAR_VALUE)
                                        .preserve_depth()
                                        .label("ocean-pass");
                                    let mut pass =
                                        frame_encoder.begin_render_pass(&ocean_pass_builder);
                                    ocean.render(&mut pass);
                                }
                            }

                            // === Pass 1.5: Atmosphere scattering (additive over planet) ===
                            if let (Some(atmo_renderer), Some(atmo_bg)) =
                                (&self.atmosphere_renderer, &self.atmosphere_bind_group)
                            {
                                // Compute atmosphere uniforms from current camera state
                                let planet_radius = self
                                    .planet_faces
                                    .as_ref()
                                    .map(|p| p.planet_radius as f32)
                                    .unwrap_or(200.0);
                                let aspect = self.surface_width() as f32
                                    / self.surface_height().max(1) as f32;
                                let orbit_angle = self.camera_time * 0.3;
                                let vp = create_orbit_camera(
                                    planet_radius,
                                    planet_radius * 0.8,
                                    orbit_angle,
                                    0.4,
                                    aspect,
                                );
                                let inv_vp = vp.inverse();

                                // Camera position: extract from orbit
                                let cam_dist = planet_radius + planet_radius * 0.8;
                                let cam_pos = glam::Vec3::new(
                                    (orbit_angle.cos() * 0.4_f64.cos() * cam_dist as f64) as f32,
                                    (0.4_f64.sin() * cam_dist as f64) as f32,
                                    (orbit_angle.sin() * 0.4_f64.cos() * cam_dist as f64) as f32,
                                );

                                // Sun direction from day/night cycle
                                let sun_dir = self.day_night.sun_direction;

                                atmo_renderer.update_uniform(
                                    &gpu.queue,
                                    glam::Vec3::ZERO, // planet center at origin
                                    sun_dir,
                                    cam_pos,
                                    inv_vp,
                                    0.1,
                                    10000.0,
                                );

                                // Atmosphere pass: additive blend, no depth write
                                let atmo_pass_builder = RenderPassBuilder::new()
                                    .preserve_color()
                                    .label("atmosphere-pass");
                                {
                                    let mut atmo_pass =
                                        frame_encoder.begin_render_pass(&atmo_pass_builder);
                                    atmo_renderer.render(&mut atmo_pass, atmo_bg);
                                }
                            }

                            // === Pass 2: Demo scene (preserves planet, clears depth) ===
                            let pass_builder = if let Some(depth_buffer) = &self.depth_buffer {
                                RenderPassBuilder::new()
                                    .preserve_color()
                                    .depth(depth_buffer.view.clone(), DepthBuffer::CLEAR_VALUE)
                                    .label("demo-scene-pass")
                            } else {
                                RenderPassBuilder::new().preserve_color()
                            };

                            {
                                let mut render_pass =
                                    frame_encoder.begin_render_pass(&pass_builder);

                                // Render both triangles
                                if let (Some(pipeline), Some(bind_group)) =
                                    (&self.unlit_pipeline, &self.camera_bind_group)
                                {
                                    if let Some(back_mesh) = &self.back_triangle_mesh {
                                        draw_unlit(
                                            &mut render_pass,
                                            pipeline,
                                            bind_group,
                                            back_mesh,
                                        );
                                    }

                                    if let Some(front_mesh) = &self.triangle_mesh {
                                        draw_unlit(
                                            &mut render_pass,
                                            pipeline,
                                            bind_group,
                                            front_mesh,
                                        );
                                    }
                                }

                                // Render textured checkerboard quad
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
                // Clear per-frame transient input state after all systems have run.
                self.keyboard_state.clear_transients();
                self.mouse_state.clear_transients();

                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        if let winit::event::DeviceEvent::MouseMotion { delta } = event {
            self.mouse_state.on_raw_motion(delta.0, delta.1);
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

/// Creates an event loop and runs the application with keyboard state forwarded
/// to the custom update callback each simulation tick.
///
/// This function blocks until the window is closed.
#[instrument(skip_all)]
pub fn run_with_config_and_input<T>(config: Config, mut custom_state: T)
where
    T: FnMut(f64, &nebula_input::KeyboardState, &nebula_input::MouseState) + 'static,
{
    let event_loop = EventLoop::new().expect("Failed to create event loop");
    let mut app = AppState::with_config(config);

    app.custom_input_update = Some(Box::new(move |dt, kb, ms| {
        custom_state(dt, kb, ms);
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
