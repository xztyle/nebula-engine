//! Demo binary that opens a Nebula Engine window with GPU-cleared background.
//!
//! Configuration is loaded from `config.ron` and can be overridden via CLI flags.
//! Run with `cargo run -p nebula-demo` to see the window.
//! Run with `cargo run -p nebula-demo -- --width 1920 --height 1080` to override size.

use clap::Parser;
use nebula_config::{CliArgs, Config};
use nebula_coords::{EntityId, SectorCoord, SpatialEntity, SpatialHashMap, WorldPosition};
use nebula_render::{BufferAllocator, IndexData, ShaderLibrary, VertexPositionColor, load_shader};
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

/// Demonstrates the buffer management system by creating triangle vertex data and uploading it to GPU buffers.
fn demonstrate_buffer_management() {
    info!("Starting buffer management demonstration");

    // Create a headless GPU context for testing buffer allocation
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

    info!("GPU device initialized for buffer management demo");

    // Create a buffer allocator
    let allocator = BufferAllocator::new(&device);
    info!("BufferAllocator created");

    // Create triangle vertex data
    let vertices: &[VertexPositionColor] = &[
        VertexPositionColor {
            position: [0.0, 0.5, 0.0],
            color: [1.0, 0.0, 0.0, 1.0],
        }, // Red top
        VertexPositionColor {
            position: [-0.5, -0.5, 0.0],
            color: [0.0, 1.0, 0.0, 1.0],
        }, // Green left
        VertexPositionColor {
            position: [0.5, -0.5, 0.0],
            color: [0.0, 0.0, 1.0, 1.0],
        }, // Blue right
    ];

    let indices: &[u16] = &[0, 1, 2];

    info!(
        "Creating triangle mesh with {} vertices and {} indices",
        vertices.len(),
        indices.len()
    );

    // Create mesh buffer using the buffer allocator
    let mesh_buffer = allocator.create_mesh(
        "demo-triangle",
        bytemuck::cast_slice(vertices),
        IndexData::U16(indices),
    );

    info!(
        "Triangle mesh buffer created successfully: {} indices, format: {:?}",
        mesh_buffer.index_count, mesh_buffer.index_format
    );

    // Create a simple quad to test u32 indices
    let quad_vertices: &[VertexPositionColor] = &[
        VertexPositionColor {
            position: [-1.0, 1.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        }, // Top-left
        VertexPositionColor {
            position: [1.0, 1.0, 0.0],
            color: [1.0, 1.0, 0.0, 1.0],
        }, // Top-right
        VertexPositionColor {
            position: [1.0, -1.0, 0.0],
            color: [1.0, 0.0, 1.0, 1.0],
        }, // Bottom-right
        VertexPositionColor {
            position: [-1.0, -1.0, 0.0],
            color: [0.0, 1.0, 1.0, 1.0],
        }, // Bottom-left
    ];

    let quad_indices: &[u32] = &[0, 1, 2, 2, 3, 0]; // Two triangles

    info!(
        "Creating quad mesh with {} vertices and {} indices (u32)",
        quad_vertices.len(),
        quad_indices.len()
    );

    let quad_mesh_buffer = allocator.create_mesh(
        "demo-quad",
        bytemuck::cast_slice(quad_vertices),
        IndexData::U32(quad_indices),
    );

    info!(
        "Quad mesh buffer created successfully: {} indices, format: {:?}",
        quad_mesh_buffer.index_count, quad_mesh_buffer.index_format
    );

    // Test empty mesh handling
    let empty_mesh = allocator.create_mesh("demo-empty", &[], IndexData::U16(&[]));

    info!(
        "Empty mesh buffer created successfully: {} indices",
        empty_mesh.index_count
    );

    info!("Buffer management demonstration completed successfully");

    // Demonstrate shader module loading
    info!("Starting shader module loading demonstration");

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
    info!("Shader module loading demonstration completed successfully");
}

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

    // Demonstrate buffer management functionality
    demonstrate_buffer_management();

    // Log initial state
    let mut demo_state = DemoState::new();
    let initial_sector = SectorCoord::from_world(&demo_state.position);

    // Update window title to show nearby count
    config.window.title = format!(
        "Nebula Engine - Nearby: {} entities",
        demo_state.nearby_count
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
