//! Demo binary that opens a Nebula Engine window with GPU-cleared background.
//!
//! Configuration is loaded from `config.ron` and can be overridden via CLI flags.
//! Run with `cargo run -p nebula-demo` to see the window.
//! Run with `cargo run -p nebula-demo -- --width 1920 --height 1080` to override size.

mod cubesphere_demos;

use clap::Parser;
use nebula_config::{CliArgs, Config};
use nebula_coords::{EntityId, SectorCoord, SpatialEntity, SpatialHashMap, WorldPosition};
use nebula_cubesphere::PlanetDef;
use nebula_render::{Aabb, Camera, DrawBatch, DrawCall, FrustumCuller, ShaderLibrary, load_shader};
use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256StarStar;
use tracing::info;

// Demo entity struct for spatial hash testing
#[derive(Debug, Clone)]
struct DemoEntity {
    id: EntityId,
    position: WorldPosition,
}

impl DemoEntity {
    fn new(id: u64, position: WorldPosition) -> Self {
        Self {
            id: EntityId::new(id),
            position,
        }
    }
}

impl SpatialEntity for DemoEntity {
    fn entity_id(&self) -> EntityId {
        self.id
    }

    fn world_position(&self) -> &WorldPosition {
        &self.position
    }
}

struct DemoState {
    position: WorldPosition,
    last_sector: Option<(i128, i128, i128)>,
    velocity: WorldPosition, // mm per second
    time_accumulator: f64,
    spatial_hash: SpatialHashMap<DemoEntity>,
    nearby_count: usize,
}

impl DemoState {
    fn new() -> Self {
        let mut spatial_hash = SpatialHashMap::new();
        let mut rng = Xoshiro256StarStar::seed_from_u64(42); // Fixed seed for reproducible demo

        // Insert 1000 entities at random positions within a reasonable range
        // around our starting position
        let center = WorldPosition::new(1_000_000, 2_000_000, 500_000);
        let spread = 10_000_000; // 10 km spread

        for id in 0..1000 {
            let x_offset = rng.gen_range(-spread..=spread);
            let y_offset = rng.gen_range(-spread..=spread);
            let z_offset = rng.gen_range(-spread..=spread);

            let entity_pos = WorldPosition::new(
                center.x + x_offset,
                center.y + y_offset,
                center.z + z_offset,
            );

            let entity = DemoEntity::new(id, entity_pos);
            spatial_hash.insert(entity);
        }

        info!(
            "Inserted {} entities into spatial hash",
            spatial_hash.count()
        );

        // Initial query for nearby entities
        let nearby = spatial_hash.query_radius(&center, 100_000); // 100m radius
        let nearby_count = nearby.len();

        Self {
            position: center,
            last_sector: None,
            // Move at ~4.3 km/s which will cross sector boundaries regularly
            // (sector size is ~4,295 km)
            velocity: WorldPosition::new(4_300_000, 0, 0), // 4.3 million mm/s = 4.3 km/s
            time_accumulator: 0.0,
            spatial_hash,
            nearby_count,
        }
    }

    fn update(&mut self, dt: f64) {
        self.time_accumulator += dt;

        // Update position based on velocity and time
        let dt_ms = (dt * 1000.0) as i128; // Convert to milliseconds
        self.position.x += self.velocity.x * dt_ms / 1000; // velocity is per second
        self.position.y += self.velocity.y * dt_ms / 1000;
        self.position.z += self.velocity.z * dt_ms / 1000;

        // Query spatial hash for entities within 100m
        let nearby = self.spatial_hash.query_radius(&self.position, 100_000); // 100m = 100,000mm
        self.nearby_count = nearby.len();

        // Check for sector boundary crossing
        let sector_coord = SectorCoord::from_world(&self.position);
        let current_sector = (
            sector_coord.sector.x,
            sector_coord.sector.y,
            sector_coord.sector.z,
        );

        if let Some(last) = self.last_sector
            && last != current_sector
        {
            info!(
                "Entered sector ({}, {}, {}) - Nearby entities: {}",
                current_sector.0, current_sector.1, current_sector.2, self.nearby_count
            );
        }

        self.last_sector = Some(current_sector);
    }
}

/// Demonstrates the shader loading system.
fn demonstrate_shader_loading() {
    info!("Starting shader loading demonstration");

    // Create a headless GPU context for testing shader compilation
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    });

    let adapter = pollster::block_on(async {
        instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .expect("Failed to find adapter")
    });

    let (device, _queue) = pollster::block_on(async {
        adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                experimental_features: Default::default(),
                ..Default::default()
            })
            .await
            .expect("Failed to create device")
    });

    info!("GPU device initialized for shader loading demo");

    let mut shader_library = ShaderLibrary::new().with_shader_dir("assets/shaders");

    // Load the unlit shader from source
    let unlit_shader_source = r#"
        struct VertexInput {
            @location(0) position: vec3<f32>,
            @location(1) color: vec4<f32>,
        }

        struct VertexOutput {
            @builtin(position) clip_position: vec4<f32>,
            @location(0) color: vec4<f32>,
        }

        @vertex
        fn vs_main(input: VertexInput) -> VertexOutput {
            var out: VertexOutput;
            out.clip_position = vec4<f32>(input.position, 1.0);
            out.color = input.color;
            return out;
        }

        @fragment
        fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
            return input.color;
        }
    "#;

    match shader_library.load_from_source(&device, "unlit", unlit_shader_source) {
        Ok(_) => info!("Compiled shader: unlit (from source)"),
        Err(e) => info!("Failed to compile shader: {}", e),
    }

    // Try loading from file (this will work if the file exists)
    match load_shader!(
        shader_library,
        &device,
        "unlit_file",
        "../../../assets/shaders/unlit.wgsl"
    ) {
        Ok(_) => info!("Compiled shader: unlit.wgsl"),
        Err(e) => info!("Failed to load shader file: {}", e),
    }

    info!("Shader library contains {} shaders", shader_library.len());
    info!("Shader loading demonstration completed successfully");
}

/// Demonstrates frustum culling by scattering 100 cubes around the camera
/// and counting how many are culled.
fn demonstrate_frustum_culling() {
    use glam::Vec3;

    info!("Starting frustum culling demonstration");

    let camera = Camera::default();
    let vp = camera.view_projection_matrix();
    let culler = FrustumCuller::new(&vp);

    let mut rng = Xoshiro256StarStar::seed_from_u64(99);
    let total = 100;
    let mut culled = 0;

    for _ in 0..total {
        // Scatter cubes in a sphere of radius 50 around the camera
        let x: f32 = rng.gen_range(-50.0..50.0);
        let y: f32 = rng.gen_range(-50.0..50.0);
        let z: f32 = rng.gen_range(-50.0..50.0);
        let half = 0.5;
        let aabb = Aabb::new(
            Vec3::new(x - half, y - half, z - half),
            Vec3::new(x + half, y + half, z + half),
        );
        if !culler.is_visible(&aabb) {
            culled += 1;
        }
    }

    info!("Culled: {culled}/{total} objects");
    info!("Frustum culling demonstration completed successfully");
}

/// Demonstrates draw call batching by collecting 100 cube draw calls
/// and batching them by pipeline and material.
fn demonstrate_draw_call_batching() {
    info!("Starting draw call batching demonstration");

    let mut batch = DrawBatch::with_capacity(100);
    let mut rng = Xoshiro256StarStar::seed_from_u64(123);

    // Simulate 100 cubes with 3 pipelines and 4 materials
    let total_calls = 100;
    for i in 0..total_calls {
        let pipeline_id = rng.gen_range(0..3_u64); // 3 pipelines
        let material_id = rng.gen_range(0..4_u64); // 4 materials
        let mesh_id = 1; // all cubes share the same mesh
        batch.push(DrawCall {
            pipeline_id,
            material_id,
            mesh_id,
            instance_index: i as u32,
        });
    }

    batch.sort();

    let group_count = batch.groups().count();
    let instanced_draw_count: usize = batch.groups().map(|g| g.instanced_groups().count()).sum();

    info!(
        "Draw calls: {} batched into {} groups, {} instanced draws (was {})",
        batch.len(),
        group_count,
        instanced_draw_count,
        total_calls
    );
    info!("Draw call batching demonstration completed successfully");
}

/// Demonstrates cube-to-sphere projection by projecting points on each face
/// and verifying the sphere is well-formed.
fn main() {
    let args = CliArgs::parse();

    // Resolve config directory
    let config_dir = args.config.clone().unwrap_or_else(|| {
        dirs::config_dir()
            .expect("Failed to resolve config directory")
            .join("nebula-engine")
    });

    // Load or create config, then apply CLI overrides
    let mut config = Config::load_or_create(&config_dir).unwrap_or_else(|e| {
        eprintln!("Failed to load config: {e}, using defaults");
        Config::default()
    });
    config.apply_cli_overrides(&args);

    // Initialize logging with config and debug settings
    let log_dir = config_dir.join("logs");
    nebula_log::init_logging(Some(&log_dir), cfg!(debug_assertions), Some(&config));

    // Demonstrate shader loading functionality
    demonstrate_shader_loading();

    // Demonstrate frustum culling
    demonstrate_frustum_culling();

    // Demonstrate draw call batching
    demonstrate_draw_call_batching();

    // Demonstrate cubesphere projection
    cubesphere_demos::demonstrate_cubesphere_projection();

    // Demonstrate sphere-to-cube inverse projection
    cubesphere_demos::demonstrate_sphere_to_cube_inverse();

    // Demonstrate same-face neighbor finding
    cubesphere_demos::demonstrate_neighbor_finding();

    // Demonstrate cross-face corner neighbors
    cubesphere_demos::demonstrate_corner_neighbors();

    // Demonstrate face UV to world position conversion
    cubesphere_demos::demonstrate_face_uv_to_world();

    // Demonstrate planet definition and registry
    cubesphere_demos::demonstrate_planet_definition();

    // Log initial state
    let mut demo_state = DemoState::new();
    let initial_sector = SectorCoord::from_world(&demo_state.position);

    // Update window title to show planet info and nearby count
    let terra = PlanetDef::earth_like("Terra", WorldPosition::default(), 42);
    config.window.title = format!(
        "Nebula Engine - Planet: {}, radius={} mm - Nearby: {} entities",
        terra.name, terra.radius, demo_state.nearby_count
    );

    info!(
        "Starting demo: {}x{} \"{}\"",
        config.window.width, config.window.height, config.window.title,
    );

    info!(
        "Demo starting at sector ({}, {}, {}) with {} entities in spatial hash",
        initial_sector.sector.x,
        initial_sector.sector.y,
        initial_sector.sector.z,
        demo_state.spatial_hash.count()
    );

    nebula_app::window::run_with_config_and_update(config, move |dt: f64| {
        demo_state.update(dt);
    });
}
