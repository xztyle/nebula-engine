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
use nebula_voxel::{
    Chunk, ChunkAddress, ChunkData, ChunkManager, Transparency, VoxelTypeDef, VoxelTypeId,
    VoxelTypeRegistry,
};
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

/// Demonstrates the voxel type registry by registering a small palette.
fn demonstrate_voxel_registry() -> usize {
    info!("Starting voxel type registry demonstration");

    let mut registry = VoxelTypeRegistry::new();

    let stone = VoxelTypeDef {
        name: "stone".to_string(),
        solid: true,
        transparency: Transparency::Opaque,
        material_index: 1,
        light_emission: 0,
    };
    let dirt = VoxelTypeDef {
        name: "dirt".to_string(),
        solid: true,
        transparency: Transparency::Opaque,
        material_index: 2,
        light_emission: 0,
    };
    let grass = VoxelTypeDef {
        name: "grass".to_string(),
        solid: true,
        transparency: Transparency::Opaque,
        material_index: 3,
        light_emission: 0,
    };

    registry.register(stone).expect("failed to register stone");
    registry.register(dirt).expect("failed to register dirt");
    registry.register(grass).expect("failed to register grass");

    let count = registry.len();
    info!("Registry: {} types", count);

    // Verify air is ID 0
    let air = registry.get(nebula_voxel::VoxelTypeId(0));
    info!("  ID 0: {} (solid={})", air.name, air.solid);

    // Verify name lookup
    if let Some(stone_id) = registry.lookup_by_name("stone") {
        let stone_def = registry.get(stone_id);
        info!(
            "  Lookup 'stone': ID {} (solid={})",
            stone_id.0, stone_def.solid
        );
    }

    info!("Voxel type registry demonstration completed successfully");
    count
}

/// Demonstrates palette-compressed chunk storage.
fn demonstrate_palette_chunk() {
    info!("Starting palette-compressed chunk demonstration");

    // Create an all-air chunk (uniform, 0 bytes storage).
    let air_chunk = ChunkData::new_air();
    info!(
        "Air chunk: {} bytes (palette: {} entry, bit_width: {})",
        air_chunk.storage_bytes(),
        air_chunk.palette_len(),
        air_chunk.bit_width(),
    );

    // Create a surface-like chunk with 4 types.
    let mut surface_chunk = ChunkData::new_air();
    let stone = VoxelTypeId(1);
    let dirt = VoxelTypeId(2);
    let grass = VoxelTypeId(3);

    // Fill bottom half with stone, a dirt layer, then grass on top.
    for z in 0..32 {
        for x in 0..32 {
            for y in 0..16 {
                surface_chunk.set(x, y, z, stone);
            }
            surface_chunk.set(x, 16, z, dirt);
            surface_chunk.set(x, 17, z, grass);
        }
    }

    info!(
        "Surface chunk: {} bytes (palette: {} entries, bit_width: {})",
        surface_chunk.storage_bytes(),
        surface_chunk.palette_len(),
        surface_chunk.bit_width(),
    );

    // Compare with uncompressed size.
    let uncompressed = 32 * 32 * 32 * 2; // 65536 bytes
    let compressed = surface_chunk.storage_bytes();
    let ratio = uncompressed as f64 / compressed.max(1) as f64;
    info!(
        "Compression: {} bytes vs {} bytes uncompressed ({:.1}x savings)",
        compressed, uncompressed, ratio,
    );

    info!("Palette-compressed chunk demonstration completed successfully");
}

/// Demonstrates the high-level Chunk get/set/fill API with bounds checking.
fn demonstrate_chunk_api() {
    info!("Starting chunk get/set API demonstration");

    let mut chunk = Chunk::new();

    // Fill procedurally: stone below y=16, dirt at y=16, grass at y=17, air above.
    let stone = VoxelTypeId(1);
    let dirt = VoxelTypeId(2);
    let grass = VoxelTypeId(3);

    for z in 0u8..32 {
        for x in 0u8..32 {
            for y in 0u8..16 {
                chunk.set(x, y, z, stone);
            }
            chunk.set(x, 16, z, dirt);
            chunk.set(x, 17, z, grass);
        }
    }

    // Verify some voxels.
    assert_eq!(chunk.get(0, 0, 0), stone);
    assert_eq!(chunk.get(15, 16, 15), dirt);
    assert_eq!(chunk.get(31, 17, 31), grass);
    assert_eq!(chunk.get(0, 31, 0), VoxelTypeId(0)); // air above

    info!(
        "Chunk API: version={}, dirty=0x{:02X}, palette={}",
        chunk.version(),
        chunk.dirty_flags(),
        chunk.palette_len(),
    );

    // Test fill.
    chunk.fill(VoxelTypeId(0));
    assert_eq!(chunk.get(0, 0, 0), VoxelTypeId(0));
    assert_eq!(chunk.palette_len(), 1);

    info!(
        "After fill(Air): version={}, palette={}",
        chunk.version(),
        chunk.palette_len(),
    );

    // Out-of-bounds access is safe.
    assert_eq!(chunk.get(32, 0, 0), VoxelTypeId(0));

    info!("Chunk get/set API demonstration completed successfully");
}

/// Demonstrates chunk serialization and deserialization round-trip.
fn demonstrate_chunk_serialization() {
    info!("Starting chunk serialization demonstration");

    let stone = VoxelTypeId(1);
    let dirt = VoxelTypeId(2);
    let grass = VoxelTypeId(3);

    let mut total_bytes = 0usize;
    let chunk_count = 25;

    for i in 0..chunk_count {
        let mut chunk = ChunkData::new_air();

        // Fill a surface-like chunk (stone below y=16, dirt at y=16, grass at y=17)
        if i > 0 {
            for z in 0..32usize {
                for x in 0..32usize {
                    for y in 0..16usize {
                        chunk.set(x, y, z, stone);
                    }
                    chunk.set(x, 16, z, dirt);
                    chunk.set(x, 17, z, grass);
                }
            }
        }

        let bytes = chunk.serialize();
        total_bytes += bytes.len();

        // Round-trip integrity check
        let restored = ChunkData::deserialize(&bytes).expect("deserialize failed");
        for z in 0..32usize {
            for y in 0..32usize {
                for x in 0..32usize {
                    assert_eq!(chunk.get(x, y, z), restored.get(x, y, z));
                }
            }
        }
    }

    let avg = total_bytes / chunk_count;
    info!(
        "Serialized {} chunks: {:.1}KB total, {}B avg",
        chunk_count,
        total_bytes as f64 / 1024.0,
        avg,
    );

    info!("Chunk serialization demonstration completed successfully");
}

/// Demonstrates the chunk manager by loading a 5x5 grid of chunks.
fn demonstrate_chunk_manager() -> (usize, usize) {
    info!("Starting chunk manager demonstration");

    let mut manager = ChunkManager::new();
    let stone = VoxelTypeId(1);

    // Load a 5x5 grid of chunks on face 0.
    for cx in 0..5_i64 {
        for cz in 0..5_i64 {
            let mut chunk = Chunk::new();
            // Fill bottom layer with stone so chunks aren't empty.
            for x in 0u8..32 {
                for z in 0u8..32 {
                    chunk.set(x, 0, z, stone);
                }
            }
            // Clear dirty flags to simulate "already meshed" state.
            chunk.clear_dirty(
                nebula_voxel::MESH_DIRTY | nebula_voxel::SAVE_DIRTY | nebula_voxel::NETWORK_DIRTY,
            );
            manager.load_chunk(ChunkAddress::new(cx, 0, cz, 0), chunk);
        }
    }

    let count = manager.loaded_count();
    info!("Chunk manager: {} chunks loaded (5x5 grid)", count);

    // Modify one voxel in a single chunk to demonstrate dirty tracking.
    let target = ChunkAddress::new(2, 0, 2, 0);
    if let Some(c) = manager.get_chunk_mut(&target) {
        c.set(0, 1, 0, VoxelTypeId(2));
    }

    let dirty_count = manager.iter_dirty(nebula_voxel::MESH_DIRTY).count();
    info!("Dirty chunks: {}/{}", dirty_count, count);

    // Verify a specific chunk is accessible.
    if let Some(c) = manager.get_chunk(&target) {
        info!(
            "  Center chunk (2,0,2): palette={}, version={}",
            c.palette_len(),
            c.version(),
        );
    }

    // Unload edge chunks to simulate camera movement.
    for cz in 0..5_i64 {
        manager.unload_chunk(ChunkAddress::new(0, 0, cz, 0));
    }
    info!(
        "After unloading edge: {} chunks remain",
        manager.loaded_count()
    );

    info!("Chunk manager demonstration completed successfully");
    (count, dirty_count)
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

    // Demonstrate voxel type registry
    let voxel_type_count = demonstrate_voxel_registry();

    // Demonstrate palette-compressed chunk
    demonstrate_palette_chunk();

    // Demonstrate chunk get/set API
    demonstrate_chunk_api();

    // Demonstrate chunk serialization
    demonstrate_chunk_serialization();

    // Demonstrate chunk manager
    let (chunks_loaded, dirty_count) = demonstrate_chunk_manager();

    // Log initial state
    let mut demo_state = DemoState::new();
    let initial_sector = SectorCoord::from_world(&demo_state.position);

    // Update window title to show planet info and nearby count
    let terra = PlanetDef::earth_like("Terra", WorldPosition::default(), 42);
    config.window.title = format!(
        "Nebula Engine - Planet: {}, radius={} mm - Registry: {} types - Chunks loaded: {} - Dirty chunks: {}/{} - Nearby: {} entities",
        terra.name,
        terra.radius,
        voxel_type_count,
        chunks_loaded,
        dirty_count,
        chunks_loaded,
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
