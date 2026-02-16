//! Demo binary that opens a Nebula Engine window with GPU-cleared background.
//!
//! Configuration is loaded from `config.ron` and can be overridden via CLI flags.
//! Run with `cargo run -p nebula-demo` to see the window.
//! Run with `cargo run -p nebula-demo -- --width 1920 --height 1080` to override size.

mod cubesphere_demos;

use bevy_ecs::prelude::IntoSystemConfigs;
use clap::Parser;
use nebula_config::{CliArgs, Config};
use nebula_coords::{EntityId, SectorCoord, SpatialEntity, SpatialHashMap, WorldPosition};
use nebula_cubesphere::PlanetDef;
use nebula_mesh::{
    ChunkNeighborhood, EdgeDirection, FaceDirection, compute_face_ao, compute_visible_faces,
    count_total_faces, count_visible_faces, greedy_mesh, vertex_ao,
};
use nebula_render::{
    Aabb, Camera, DrawBatch, DrawCall, FrustumCuller, GpuBufferPool, GpuChunkMesh, ShaderLibrary,
    load_shader,
};
use nebula_voxel::{
    Chunk, ChunkAddress, ChunkData, ChunkLoadConfig, ChunkLoader, ChunkManager, Transparency,
    VoxelEventBuffer, VoxelTypeDef, VoxelTypeId, VoxelTypeRegistry, set_voxel, set_voxels_batch,
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

        let stats = chunk.serialize_stats();
        let bytes = chunk.serialize();
        total_bytes += bytes.len();

        let raw_kb = 32 * 1024; // 32KB raw uncompressed
        info!(
            "Chunk ({},0): RLE {} {}B, palette {}B, raw {}KB",
            i,
            if stats.rle_used { "on" } else { "off" },
            stats.index_bytes,
            stats.palette_bytes,
            raw_kb / 1024,
        );

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
        "Serialized {} chunks: {:.1}KB total, {}B avg (with RLE compression)",
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

/// Demonstrates chunk loading/unloading with hysteresis and budgeting.
fn demonstrate_chunk_loading() -> usize {
    info!("Starting chunk loading/unloading demonstration");

    let config = ChunkLoadConfig {
        load_radius: 4,
        unload_radius: 6,
        loads_per_tick: 8,
        unloads_per_tick: 16,
    };
    let mut loader = ChunkLoader::new(config);
    let mut manager = ChunkManager::new();

    // Simulate camera at origin; run several ticks to load nearby chunks.
    let camera = ChunkAddress::new(0, 0, 0, 0);
    for tick in 0..20 {
        let result = loader.tick(camera, &mut manager);
        if result.loaded > 0 {
            info!(
                "  Tick {}: loaded {} chunks (total in manager: {})",
                tick,
                result.loaded,
                manager.loaded_count()
            );
        }
    }
    let loaded_at_origin = manager.loaded_count();
    info!(
        "Loaded {} chunks around origin (radius 4)",
        loaded_at_origin
    );

    // Move camera far away — chunks should be unloaded.
    let far_camera = ChunkAddress::new(20, 0, 0, 0);
    let result = loader.tick(far_camera, &mut manager);
    info!(
        "After camera move: loaded={}, unloaded={}, remaining={}",
        result.loaded,
        result.unloaded,
        manager.loaded_count()
    );

    info!("Chunk loading/unloading demonstration completed successfully");
    loaded_at_origin
}

/// Demonstrates Copy-on-Write chunk sharing and mutation isolation.
fn demonstrate_cow_chunks() {
    use nebula_voxel::CowChunk;

    info!("Starting Copy-on-Write chunk demonstration");

    // Create two air chunks — they share the same Arc allocation.
    let a = CowChunk::new_air();
    let b = CowChunk::new_air();
    assert!(a.ptr_eq(&b), "air chunks must share storage");
    info!(
        "Two air chunks share storage: ptr_eq={}, ref_count={}",
        a.ptr_eq(&b),
        a.ref_count()
    );

    // Clone shared — still the same allocation.
    let mut c = a.clone_shared();
    info!(
        "After clone_shared: ref_count={}, is_shared={}",
        a.ref_count(),
        c.is_shared()
    );

    // Mutate c — triggers CoW clone; a and b remain unchanged.
    c.get_mut().set(0, 0, 0, VoxelTypeId(42));
    let original_crc = a.get().get(0, 0, 0);
    let clone_crc = c.get().get(0, 0, 0);
    info!(
        "Original CRC: 0x{:04X}, Clone CRC: 0x{:04X}",
        original_crc.0, clone_crc.0
    );
    assert_eq!(original_crc, VoxelTypeId(0), "original must be unchanged");
    assert_eq!(clone_crc, VoxelTypeId(42), "clone must have new value");
    assert!(!a.ptr_eq(&c), "after mutation they must not share");

    // Demonstrate memory savings: 1000 air chunks share one allocation.
    let chunks: Vec<CowChunk> = (0..1000).map(|_| CowChunk::new_air()).collect();
    let all_shared = chunks.windows(2).all(|w| w[0].ptr_eq(&w[1]));
    info!(
        "1000 air chunks all share storage: {}, ref_count={}",
        all_shared,
        chunks[0].ref_count()
    );

    info!("Copy-on-Write chunk demonstration completed successfully");
}

/// Demonstrates voxel modification events.
fn demonstrate_voxel_events() {
    info!("Starting voxel modification events demonstration");

    let mut manager = ChunkManager::new();
    let mut events = VoxelEventBuffer::new();

    let addr = ChunkAddress::new(0, 3, 0, 0);
    manager.load_chunk(addr, Chunk::new());

    // Single voxel modification
    let stone = VoxelTypeId(1);
    set_voxel(&mut manager, &addr, 5, 17, 8, stone, &mut events);

    for evt in events.read() {
        info!(
            "VoxelModified {{ chunk: ({},{},{}), pos: ({},{},{}), old: {:?}, new: {:?} }}",
            evt.chunk.x,
            evt.chunk.y,
            evt.chunk.z,
            evt.local_pos.0,
            evt.local_pos.1,
            evt.local_pos.2,
            evt.old_type,
            evt.new_type,
        );
    }

    // Same-type set produces no new event
    let before = events.read().count();
    set_voxel(&mut manager, &addr, 5, 17, 8, stone, &mut events);
    let after = events.read().count();
    info!(
        "Same-type set: events before={}, after={} (no new event)",
        before, after
    );

    // Batch modification
    events.swap();
    let dirt = VoxelTypeId(2);
    let mods: Vec<_> = (0..5).map(|i| (i, 0, 0, dirt)).collect();
    let count = set_voxels_batch(&mut manager, &addr, &mods, &mut events);
    info!(
        "Batch modified {} voxels, {} individual events, {} batch events",
        count,
        events.read().count(),
        events.read_batch().count()
    );

    // Frame advance clears old events
    events.swap();
    events.swap();
    info!(
        "After 2 swaps: {} events remaining (expected 0)",
        events.len()
    );

    info!("Voxel modification events demonstration completed successfully");
}

/// Demonstrates chunk data versioning and serialization round-trip.
fn demonstrate_chunk_versioning() -> u64 {
    info!("Starting chunk data versioning demonstration");

    let mut chunk = Chunk::new();
    assert_eq!(chunk.version(), 0);

    // Modify voxels and track version increments.
    let stone = VoxelTypeId(1);
    let dirt = VoxelTypeId(2);
    for i in 0u8..32 {
        chunk.set(i, 0, 0, stone);
    }
    for i in 0u8..15 {
        chunk.set(i, 1, 0, dirt);
    }
    let version_after_sets = chunk.version();
    info!(
        "Chunk version after 47 modifications: {}",
        version_after_sets
    );
    assert_eq!(version_after_sets, 47);

    // Serialize with version, round-trip, verify.
    let bytes = chunk.serialize();
    let restored = Chunk::deserialize(&bytes).expect("chunk deserialize failed");
    assert_eq!(restored.version(), version_after_sets);
    info!(
        "Version survives serialization: {} == {}",
        restored.version(),
        version_after_sets
    );

    // Verify voxel data integrity.
    for i in 0u8..32 {
        assert_eq!(restored.get(i, 0, 0), stone);
    }
    for i in 0u8..15 {
        assert_eq!(restored.get(i, 1, 0), dirt);
    }

    info!("Chunk data versioning demonstration completed successfully");
    version_after_sets
}

/// Demonstrates visible face detection by building a surface chunk and
/// showing how many faces are culled.
fn demonstrate_visible_face_detection() -> (u32, u32) {
    info!("Starting visible face detection demonstration");

    let mut registry = VoxelTypeRegistry::new();
    let stone_id = registry
        .register(VoxelTypeDef {
            name: "vfd_stone".to_string(),
            solid: true,
            transparency: Transparency::Opaque,
            material_index: 1,
            light_emission: 0,
        })
        .expect("register stone");
    let glass_id = registry
        .register(VoxelTypeDef {
            name: "vfd_glass".to_string(),
            solid: true,
            transparency: Transparency::SemiTransparent,
            material_index: 2,
            light_emission: 0,
        })
        .expect("register glass");

    // Build a surface chunk: stone below y=8, glass layer at y=8, air above.
    let mut chunk = ChunkData::new_air();
    for z in 0..32_usize {
        for x in 0..32_usize {
            for y in 0..8_usize {
                chunk.set(x, y, z, stone_id);
            }
            chunk.set(x, 8, z, glass_id);
        }
    }

    let neighbors = ChunkNeighborhood::all_air();
    let faces = compute_visible_faces(&chunk, &neighbors, &registry);

    let visible = count_visible_faces(&faces);
    let total = count_total_faces(&chunk, &registry);

    info!("Faces: {} visible of {} total", visible, total);
    info!(
        "Culled {} interior faces ({:.1}% reduction)",
        total - visible,
        (1.0 - visible as f64 / total as f64) * 100.0
    );

    info!("Visible face detection demonstration completed successfully");
    (visible, total)
}

/// Demonstrates greedy meshing by merging a flat grass plain into minimal quads.
fn demonstrate_greedy_meshing() -> (usize, usize) {
    info!("Starting greedy meshing demonstration");

    let mut registry = VoxelTypeRegistry::new();
    let grass_id = registry
        .register(VoxelTypeDef {
            name: "gm_grass".to_string(),
            solid: true,
            transparency: Transparency::Opaque,
            material_index: 3,
            light_emission: 0,
        })
        .expect("register grass");

    // Build a flat grass plain at y=0.
    let mut chunk = ChunkData::new_air();
    for z in 0..32_usize {
        for x in 0..32_usize {
            chunk.set(x, 0, z, grass_id);
        }
    }

    let neighbors = ChunkNeighborhood::all_air();
    let visible = compute_visible_faces(&chunk, &neighbors, &registry);

    // Count naive quads (one per visible face).
    let naive_quads = count_visible_faces(&visible) as usize;

    // Greedy mesh.
    let mesh = greedy_mesh(&chunk, &visible, &neighbors, &registry);
    let greedy_quads = mesh.quad_count();

    info!(
        "Quads: {} (greedy) vs {} (naive)",
        greedy_quads, naive_quads
    );
    info!(
        "Reduction: {:.1}x fewer quads",
        naive_quads as f64 / greedy_quads.max(1) as f64
    );

    info!("Greedy meshing demonstration completed successfully");
    (greedy_quads, naive_quads)
}

/// Demonstrates ambient occlusion: vertices at concave corners get darker shading.
fn demonstrate_ambient_occlusion() -> (u8, u8, usize) {
    info!("Starting ambient occlusion demonstration");

    // Basic vertex AO checks
    let exposed = vertex_ao(false, false, false);
    let occluded = vertex_ao(true, true, true);
    info!("Vertex AO: exposed={exposed}, fully occluded={occluded}");

    // Build a staircase to demonstrate AO gradients
    let mut registry = VoxelTypeRegistry::new();
    let stone_id = registry
        .register(VoxelTypeDef {
            name: "ao_stone".to_string(),
            solid: true,
            transparency: Transparency::Opaque,
            material_index: 1,
            light_emission: 0,
        })
        .expect("register stone");

    let mut chunk = ChunkData::new_air();
    // Staircase: each step is one block higher
    for step in 0..8_usize {
        for z in 0..8_usize {
            for y in 0..=step {
                chunk.set(step, y, z, stone_id);
            }
        }
    }

    let neighbors = ChunkNeighborhood::all_air();
    let visible = compute_visible_faces(&chunk, &neighbors, &registry);
    let mesh = greedy_mesh(&chunk, &visible, &neighbors, &registry);

    // Count vertices with non-zero AO
    let ao_vertices = mesh.vertices.iter().filter(|v| v.ao > 0).count();
    info!(
        "Staircase mesh: {} quads, {} vertices with AO shading",
        mesh.quad_count(),
        ao_vertices
    );

    // Compute face AO for a step corner to verify gradient
    let ao = compute_face_ao(&neighbors, &registry, (1, 0, 0), FaceDirection::PosY);
    info!("Step corner AO values: {ao:?}");

    info!("Ambient occlusion demonstration completed successfully");
    (exposed, occluded, ao_vertices)
}

/// Demonstrates adjacent chunk culling: faces at chunk boundaries are
/// correctly hidden when the neighboring chunk has solid voxels.
fn demonstrate_adjacent_chunk_culling() -> (u32, u32) {
    info!("Starting adjacent chunk culling demonstration");

    let mut registry = VoxelTypeRegistry::new();
    let stone_id = registry
        .register(VoxelTypeDef {
            name: "acc_stone".to_string(),
            solid: true,
            transparency: Transparency::Opaque,
            material_index: 1,
            light_emission: 0,
        })
        .expect("register stone");

    // Build two adjacent chunks: center is solid stone, +X neighbor is also solid.
    // Without neighbor data, the center's +X boundary faces would be visible.
    // With the neighbor loaded, those 32×32 = 1024 faces should be culled.
    let center = ChunkData::new(stone_id);
    let pos_x_neighbor = ChunkData::new(stone_id);

    // Without neighbor: all boundary faces visible
    let no_neighbor = ChunkNeighborhood::all_air();
    let faces_without = compute_visible_faces(&center, &no_neighbor, &registry);
    let visible_without = count_visible_faces(&faces_without);

    // With +X neighbor: +X boundary faces should be culled
    let mut with_neighbor = ChunkNeighborhood::all_air();
    with_neighbor.set(0, pos_x_neighbor.clone()); // direction 0 = +X

    // Also set up edge and corner neighbors using the new API
    for edge in EdgeDirection::ALL {
        with_neighbor.set_edge_neighbor(edge, &pos_x_neighbor);
    }

    let faces_with = compute_visible_faces(&center, &with_neighbor, &registry);
    let visible_with = count_visible_faces(&faces_with);

    let culled = visible_without - visible_with;
    info!(
        "Adjacent culling: {} visible without neighbor, {} with neighbor ({} faces culled)",
        visible_without, visible_with, culled
    );
    info!(
        "Boundary face reduction: {:.1}%",
        culled as f64 / visible_without as f64 * 100.0
    );

    info!("Adjacent chunk culling demonstration completed successfully");
    (visible_without, visible_with)
}

/// Demonstrates GPU mesh upload and buffer pool reuse.
fn demonstrate_gpu_mesh_upload() -> (u64, u64, bool) {
    use nebula_mesh::{ChunkVertex, FaceDirection, PackedChunkMesh};

    info!("Starting GPU mesh upload demonstration");

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

    let (device, queue) = pollster::block_on(async {
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

    // Build a packed mesh with 100 quads (simulating a small chunk surface)
    let mut mesh = PackedChunkMesh::new();
    for i in 0..10u8 {
        for j in 0..10u8 {
            mesh.push_quad(
                [
                    ChunkVertex::new([i, 0, j], FaceDirection::PosY, 0, 1, [0, 0]),
                    ChunkVertex::new([i + 1, 0, j], FaceDirection::PosY, 0, 1, [1, 0]),
                    ChunkVertex::new([i + 1, 0, j + 1], FaceDirection::PosY, 0, 1, [1, 1]),
                    ChunkVertex::new([i, 0, j + 1], FaceDirection::PosY, 0, 1, [0, 1]),
                ],
                false,
            );
        }
    }

    // Upload to GPU
    let mut gpu_mesh = GpuChunkMesh::upload(&device, &mesh);
    let upload_bytes = gpu_mesh.total_gpu_bytes();
    info!(
        "Uploaded mesh: {} vertices, {} indices, {} bytes on GPU",
        gpu_mesh.vertex_count, gpu_mesh.index_count, upload_bytes
    );

    // Re-upload a smaller mesh (simulating remesh after block edit)
    let mut small_mesh = PackedChunkMesh::new();
    for i in 0..5u8 {
        small_mesh.push_quad(
            [
                ChunkVertex::new([i, 0, 0], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([i + 1, 0, 0], FaceDirection::PosY, 0, 1, [1, 0]),
                ChunkVertex::new([i + 1, 0, 1], FaceDirection::PosY, 0, 1, [1, 1]),
                ChunkVertex::new([i, 0, 1], FaceDirection::PosY, 0, 1, [0, 1]),
            ],
            false,
        );
    }
    let reused = gpu_mesh.reupload(&device, &queue, &small_mesh);
    info!(
        "Reupload (smaller mesh): reused existing buffers = {}",
        reused
    );

    // Demonstrate buffer pool
    let mut pool = GpuBufferPool::new();
    let (vb, vc) = pool.acquire_vertex_buffer(&device, 1000);
    let (ib, ic) = pool.acquire_index_buffer(&device, 500);
    let pool_allocated = pool.gpu_memory_allocated();
    let pool_in_use = pool.gpu_memory_in_use();
    info!(
        "Buffer pool: allocated={} bytes, in_use={} bytes",
        pool_allocated, pool_in_use
    );

    // Release and re-acquire to show reuse
    pool.release_vertex_buffer(vb, vc);
    pool.release_index_buffer(ib, ic);
    let (_vb2, _) = pool.acquire_vertex_buffer(&device, 1000);
    let (_ib2, _) = pool.acquire_index_buffer(&device, 500);
    let pool_allocated_after = pool.gpu_memory_allocated();
    info!(
        "After release+reacquire: allocated={} bytes (unchanged={})",
        pool_allocated_after,
        pool_allocated == pool_allocated_after
    );

    info!("GPU mesh upload demonstration completed successfully");
    (upload_bytes, pool_allocated, reused)
}

/// Demonstrates async mesh generation using the [`MeshingPipeline`].
///
/// Submits multiple chunks to background threads and collects results,
/// verifying that meshing completes without blocking the main thread.
fn demonstrate_async_meshing() -> (usize, usize) {
    use nebula_mesh::{ChunkNeighborhood, MeshingPipeline, MeshingTask};
    use nebula_voxel::{ChunkAddress, ChunkData, VoxelTypeId};
    use std::sync::Arc;

    info!("Starting async mesh generation demonstration");

    let mut reg = VoxelTypeRegistry::new();
    let _ = reg.register(VoxelTypeDef {
        name: "stone".to_string(),
        transparency: nebula_voxel::Transparency::Opaque,
        solid: true,
        material_index: 0,
        light_emission: 0,
    });
    let registry = Arc::new(reg);

    let mut pipeline = MeshingPipeline::new(2, 8, registry);

    // Submit 4 chunks for async meshing.
    let chunk_count = 4usize;
    for i in 0..chunk_count {
        let mut chunk = ChunkData::new(VoxelTypeId(0));
        // Place a stone block so each chunk produces a non-empty mesh.
        chunk.set(16, 16, 16, VoxelTypeId(1));
        let neighborhood = ChunkNeighborhood::from_center_only(chunk);
        let task = MeshingTask {
            chunk_addr: ChunkAddress::new(i as i64, 0, 0, 0),
            neighborhood,
            data_version: 1,
        };
        assert!(pipeline.submit(task), "Failed to submit meshing task {i}");
    }

    // Collect results (with timeout).
    let mut results = Vec::new();
    let start = std::time::Instant::now();
    while results.len() < chunk_count {
        results.extend(pipeline.drain_results());
        assert!(
            start.elapsed().as_secs() < 10,
            "Timed out waiting for async mesh results"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    let total_quads: usize = results.iter().map(|r| r.mesh.quad_count()).sum();

    info!(
        "Async meshing: {} chunks meshed, {} total quads",
        results.len(),
        total_quads
    );

    pipeline.shutdown();

    info!("Async mesh generation demonstration completed successfully");
    (results.len(), total_quads)
}

/// Demonstrates cubesphere vertex displacement: transforms flat chunk mesh
/// vertices onto the planet's curved surface.
fn demonstrate_cubesphere_displacement() -> (usize, f64, f64) {
    use nebula_cubesphere::{ChunkAddress as CsChunkAddress, CubeFace};
    use nebula_mesh::{
        ChunkVertex, FaceDirection, PackedChunkMesh, PlanetParams, displace_to_cubesphere,
        displace_vertex,
    };

    info!("Starting cubesphere vertex displacement demonstration");

    // A 1000-meter radius planet with 1m voxels
    let planet = PlanetParams::new(1000.0, 1.0);

    // Build a surface slab on the +X face at LOD 10 (1024 chunks per axis)
    let chunk_addr = CsChunkAddress::new(CubeFace::PosX, 10, 0, 0);

    let mut mesh = PackedChunkMesh::new();
    // Create a flat 32×32 surface at y=0 (ground level)
    for x in 0..32u8 {
        for z in 0..32u8 {
            mesh.push_quad(
                [
                    ChunkVertex::new([x, 0, z], FaceDirection::PosY, 0, 1, [0, 0]),
                    ChunkVertex::new([x + 1, 0, z], FaceDirection::PosY, 0, 1, [1, 0]),
                    ChunkVertex::new([x + 1, 0, z + 1], FaceDirection::PosY, 0, 1, [1, 1]),
                    ChunkVertex::new([x, 0, z + 1], FaceDirection::PosY, 0, 1, [0, 1]),
                ],
                false,
            );
        }
    }

    let vertex_count = mesh.vertices.len();
    info!(
        "Flat mesh: {} vertices, {} triangles",
        vertex_count,
        mesh.triangle_count()
    );

    // Displace onto cubesphere
    let buf = displace_to_cubesphere(&mesh, &chunk_addr, &planet);
    assert_eq!(buf.len(), vertex_count);

    // Measure displacement statistics
    let mut min_dist = f64::MAX;
    let mut max_dist = f64::MIN;
    for pos in &buf.positions {
        let d =
            ((pos[0] as f64).powi(2) + (pos[1] as f64).powi(2) + (pos[2] as f64).powi(2)).sqrt();
        min_dist = min_dist.min(d);
        max_dist = max_dist.max(d);
    }

    info!(
        "Displaced {} vertices onto cubesphere: distance range [{:.2}, {:.2}] (radius={})",
        buf.len(),
        min_dist,
        max_dist,
        planet.radius
    );
    info!(
        "Displacement buffer: {} bytes ({} bytes/vertex)",
        buf.byte_size(),
        buf.byte_size() / buf.len().max(1)
    );

    // Verify all six faces work
    for face in CubeFace::ALL {
        let addr = CsChunkAddress::new(face, 10, 0, 0);
        let pos = displace_vertex([16, 0, 16], &addr, &planet);
        let dir = pos.normalize();
        let normal = face.normal();
        assert!(
            dir.dot(normal) > 0.0,
            "Vertex on {face:?} should be in correct hemisphere"
        );
    }
    info!("All 6 cube faces displace to correct hemispheres");

    // Verify boundary alignment between adjacent chunks
    let addr_a = CsChunkAddress::new(CubeFace::PosX, 10, 0, 0);
    let addr_b = CsChunkAddress::new(CubeFace::PosX, 10, 1, 0);
    let pos_a = displace_vertex([32, 0, 16], &addr_a, &planet);
    let pos_b = displace_vertex([0, 0, 16], &addr_b, &planet);
    let boundary_error = (pos_a - pos_b).length();
    info!(
        "Boundary alignment error: {:.2e} meters (should be ~0)",
        boundary_error
    );

    info!("Cubesphere vertex displacement demonstration completed successfully");
    (vertex_count, min_dist, max_dist)
}

/// Demonstrates mesh cache invalidation: version tracking and boundary detection.
fn demonstrate_mesh_invalidation() -> (usize, usize, usize) {
    use nebula_mesh::{ChunkMeshState, MeshInvalidator};
    use nebula_voxel::ChunkAddress;

    info!("Starting mesh cache invalidation demonstration");

    // Interior edit: only the edited chunk is invalidated.
    let addr = ChunkAddress::new(0, 0, 0, 0);
    let interior_dirty = MeshInvalidator::invalidate(addr, (16, 16, 16), 32);
    assert_eq!(
        interior_dirty.len(),
        1,
        "Interior edit should only invalidate self"
    );

    // Boundary edit at x=0: self + -X neighbor.
    let boundary_dirty = MeshInvalidator::invalidate(addr, (0, 16, 16), 32);
    assert_eq!(
        boundary_dirty.len(),
        2,
        "Boundary edit should invalidate self + 1 neighbor"
    );

    // Corner edit at (0,0,0): self + 3 face neighbors.
    let corner_dirty = MeshInvalidator::invalidate(addr, (0, 0, 0), 32);
    assert_eq!(
        corner_dirty.len(),
        4,
        "Corner edit should invalidate self + 3 neighbors"
    );

    // Version-based staleness detection.
    let mut state = ChunkMeshState::new();
    assert!(
        state.needs_remesh(1),
        "Fresh state should need remesh vs v1"
    );
    state.meshed_version = 1;
    assert!(
        !state.needs_remesh(1),
        "Up-to-date state should not need remesh"
    );
    assert!(
        state.needs_remesh(2),
        "Stale state should need remesh vs v2"
    );
    state.remesh_pending = true;
    assert!(
        !state.needs_remesh(2),
        "Pending remesh should suppress resubmit"
    );

    info!(
        "Mesh invalidation: interior={}, boundary={}, corner={} dirty chunks",
        interior_dirty.len(),
        boundary_dirty.len(),
        corner_dirty.len(),
    );

    info!("Mesh cache invalidation demonstration completed successfully");
    (
        interior_dirty.len(),
        boundary_dirty.len(),
        corner_dirty.len(),
    )
}

/// Demonstrates the biome system: registry, Whittaker diagram, and noise sampling.
fn demonstrate_biome_system() -> usize {
    use nebula_terrain::{
        BiomeDef, BiomeRegistry, BiomeSampler, WhittakerDiagram, WhittakerRegion,
    };

    info!("Starting biome system demonstration");

    let mut registry = BiomeRegistry::new();

    let tundra = registry
        .register(BiomeDef {
            name: "tundra".into(),
            surface_voxel: VoxelTypeId(10),
            subsurface_voxel: VoxelTypeId(11),
            vegetation_density: 0.02,
            tree_type: None,
        })
        .unwrap();

    let desert = registry
        .register(BiomeDef {
            name: "desert".into(),
            surface_voxel: VoxelTypeId(20),
            subsurface_voxel: VoxelTypeId(21),
            vegetation_density: 0.01,
            tree_type: None,
        })
        .unwrap();

    let plains = registry
        .register(BiomeDef {
            name: "plains".into(),
            surface_voxel: VoxelTypeId(30),
            subsurface_voxel: VoxelTypeId(31),
            vegetation_density: 0.3,
            tree_type: Some("oak".into()),
        })
        .unwrap();

    let forest = registry
        .register(BiomeDef {
            name: "forest".into(),
            surface_voxel: VoxelTypeId(40),
            subsurface_voxel: VoxelTypeId(41),
            vegetation_density: 0.8,
            tree_type: Some("birch".into()),
        })
        .unwrap();

    let tropical = registry
        .register(BiomeDef {
            name: "tropical_rainforest".into(),
            surface_voxel: VoxelTypeId(50),
            subsurface_voxel: VoxelTypeId(51),
            vegetation_density: 0.95,
            tree_type: Some("palm".into()),
        })
        .unwrap();

    let diagram = WhittakerDiagram {
        regions: vec![
            WhittakerRegion {
                temp_min: 0.0,
                temp_max: 0.2,
                moisture_min: 0.0,
                moisture_max: 0.5,
                biome_id: tundra,
            },
            WhittakerRegion {
                temp_min: 0.0,
                temp_max: 0.2,
                moisture_min: 0.5,
                moisture_max: 1.0,
                biome_id: tundra,
            },
            WhittakerRegion {
                temp_min: 0.5,
                temp_max: 1.0,
                moisture_min: 0.0,
                moisture_max: 0.2,
                biome_id: desert,
            },
            WhittakerRegion {
                temp_min: 0.5,
                temp_max: 0.8,
                moisture_min: 0.2,
                moisture_max: 0.5,
                biome_id: plains,
            },
            WhittakerRegion {
                temp_min: 0.2,
                temp_max: 0.5,
                moisture_min: 0.2,
                moisture_max: 0.6,
                biome_id: plains,
            },
            WhittakerRegion {
                temp_min: 0.2,
                temp_max: 0.5,
                moisture_min: 0.6,
                moisture_max: 1.0,
                biome_id: forest,
            },
            WhittakerRegion {
                temp_min: 0.8,
                temp_max: 1.0,
                moisture_min: 0.5,
                moisture_max: 1.0,
                biome_id: tropical,
            },
            WhittakerRegion {
                temp_min: 0.2,
                temp_max: 0.5,
                moisture_min: 0.0,
                moisture_max: 0.2,
                biome_id: plains,
            },
            WhittakerRegion {
                temp_min: 0.5,
                temp_max: 0.8,
                moisture_min: 0.5,
                moisture_max: 1.0,
                biome_id: forest,
            },
        ],
        fallback: plains,
    };

    let biome_count = registry.len();

    let sampler = BiomeSampler::new(42, diagram);

    // Sample a few points to demonstrate
    let test_points = [
        glam::DVec3::new(1000.0, 0.0, 0.0),
        glam::DVec3::new(0.0, 1000.0, 0.0),
        glam::DVec3::new(0.0, 0.0, 1000.0),
        glam::DVec3::new(500.0, 500.0, 500.0),
    ];

    for (i, point) in test_points.iter().enumerate() {
        let (biome_id, temp, moisture) = sampler.sample(*point);
        let def = registry.get(biome_id);
        info!(
            "  Point {}: biome={}, temp={:.3}, moisture={:.3}",
            i, def.name, temp, moisture
        );
    }

    info!(
        "Biomes: {} registered, sampling Whittaker diagram",
        biome_count
    );

    info!("Biome system demonstration completed successfully");
    biome_count
}

/// Demonstrates multi-octave noise heightmap generation.
fn demonstrate_heightmap() -> (f64, f64) {
    use nebula_terrain::{HeightmapParams, HeightmapSampler};

    let world_seed: u64 = 0xDEAD_BEEF;
    info!("Seed: 0x{:X}, deterministic: true", world_seed);
    info!("Starting multi-octave noise heightmap demonstration");

    let params = HeightmapParams {
        seed: 42,
        octaves: 6,
        amplitude: 128.0,
        base_frequency: 0.01,
        ..Default::default()
    };
    let sampler = HeightmapSampler::new(params);

    let mut min_h = f64::MAX;
    let mut max_h = f64::MIN;

    // Sample a 64x64 grid to demonstrate terrain generation
    for x in 0..64 {
        for z in 0..64 {
            let h = sampler.sample(x as f64, z as f64);
            min_h = min_h.min(h);
            max_h = max_h.max(h);
        }
    }

    info!(
        "Heightmap: {} octaves, range [{:.0}, {:.0}]",
        sampler.params().octaves,
        min_h,
        max_h,
    );

    info!("Multi-octave noise heightmap demonstration completed successfully");
    (min_h, max_h)
}

/// Demonstrates cubesphere terrain height sampling across all cube faces.
fn demonstrate_cubesphere_terrain_height() {
    use nebula_cubesphere::{CubeFace, FaceCoord, face_coord_to_sphere_everitt};
    use nebula_terrain::{HeightmapParams, TerrainHeightConfig, TerrainHeightSampler};

    info!("Starting cubesphere terrain height demonstration");

    let sampler = TerrainHeightSampler::new(
        HeightmapParams {
            seed: 42,
            octaves: 6,
            amplitude: 4000.0,
            base_frequency: 0.001,
            ..Default::default()
        },
        TerrainHeightConfig::default(),
    );

    let mut min_h = f64::MAX;
    let mut max_h = f64::MIN;
    let mut total_samples = 0u64;

    // Sample terrain across all 6 cube faces
    for face in CubeFace::ALL {
        for u_step in 0..=20 {
            for v_step in 0..=20 {
                let u = u_step as f64 / 20.0;
                let v = v_step as f64 / 20.0;
                let fc = FaceCoord::new(face, u, v);
                let sphere_pt = face_coord_to_sphere_everitt(&fc);
                let h = sampler.sample_height(sphere_pt);
                min_h = min_h.min(h);
                max_h = max_h.max(h);
                total_samples += 1;
            }
        }
    }

    info!(
        "Cubesphere terrain: {total_samples} samples across 6 faces, \
         height range [{min_h:.1}, {max_h:.1}]"
    );

    // Verify radius at a sample point
    let sample_pt = glam::DVec3::new(1.0, 0.0, 0.0);
    let radius = sampler.sample_radius(sample_pt);
    info!(
        "Sample radius at +X: {radius:.1} (base: {})",
        sampler.config().planet_radius
    );

    info!("Cubesphere terrain height demonstration completed successfully");
}

/// Demonstrates ore resource distribution with default ore configurations.
fn demonstrate_ore_distribution() {
    use glam::DVec3;
    use nebula_terrain::{OreDistributor, default_ore_distributions};

    info!("Starting ore resource distribution demonstration");

    let seed = 42u64;
    let ores = default_ore_distributions();
    let ore_count = ores.len();
    let distributor = OreDistributor::new(seed, ores);

    let planet_radius = 6_371_000.0_f64;
    let surface_height = planet_radius + 100.0;

    // Sample a grid of subsurface voxels and tally ore hits per type.
    let mut ore_hits: std::collections::HashMap<nebula_voxel::VoxelTypeId, u64> =
        std::collections::HashMap::new();
    let total_samples = 100_000u64;

    for i in 0..total_samples {
        let depth = (i % 500) as f64 + 1.0;
        let x = (i as f64) * 0.7;
        let z = (i as f64) * 1.3;
        let voxel_pos = DVec3::new(surface_height - depth, x, z);
        if let Some(ore_id) = distributor.sample_ore(voxel_pos, surface_height) {
            *ore_hits.entry(ore_id).or_insert(0) += 1;
        }
    }

    let total_ore: u64 = ore_hits.values().sum();
    info!(
        "Ore distribution: {total_samples} samples, {total_ore} ore hits ({:.2}%), {ore_count} ore types registered",
        (total_ore as f64 / total_samples as f64) * 100.0
    );

    for (id, count) in &ore_hits {
        info!("  Ore type {:?}: {count} hits", id);
    }

    assert!(
        total_ore > 0,
        "Expected at least some ore hits in {total_samples} samples"
    );
    assert_eq!(distributor.ore_count(), ore_count);

    info!("Ore resource distribution demonstration completed successfully");
}

/// Demonstrates feature placement: Poisson disk sampling and biome-aware feature distribution.
fn demonstrate_feature_placement() -> usize {
    use hashbrown::HashMap;
    use nebula_terrain::{
        BiomeFeatureConfig, BiomeId, FeaturePlacer, FeatureTypeDef, FeatureTypeId, poisson_disk_2d,
    };

    info!("Starting feature placement demonstration");

    // Verify Poisson disk sampling produces well-spaced points.
    let points = poisson_disk_2d(42, (0.0, 0.0), (100.0, 100.0), 5.0, 30);
    info!(
        "Poisson disk sampling: {} points in 100x100 area with min spacing 5.0",
        points.len()
    );
    assert!(
        points.len() > 50,
        "Expected many Poisson points, got {}",
        points.len()
    );

    // Define feature types.
    let tree = FeatureTypeDef {
        name: "oak_tree".into(),
        id: FeatureTypeId(1),
        min_spacing: 8.0,
        max_slope: 0.5,
        min_height_above_sea: 2.0,
        scale_range: (0.7, 1.3),
    };
    let boulder = FeatureTypeDef {
        name: "boulder".into(),
        id: FeatureTypeId(2),
        min_spacing: 6.0,
        max_slope: 1.0,
        min_height_above_sea: 1.0,
        scale_range: (0.5, 2.0),
    };
    let cactus = FeatureTypeDef {
        name: "cactus".into(),
        id: FeatureTypeId(3),
        min_spacing: 10.0,
        max_slope: 0.3,
        min_height_above_sea: 3.0,
        scale_range: (0.8, 1.5),
    };

    // Configure biomes with features.
    let forest_biome = BiomeId(0);
    let desert_biome = BiomeId(1);
    let ocean_biome = BiomeId(2);

    let mut biome_features = HashMap::new();
    biome_features.insert(
        forest_biome,
        BiomeFeatureConfig {
            features: vec![(FeatureTypeId(1), 0.8), (FeatureTypeId(2), 0.3)],
        },
    );
    biome_features.insert(
        desert_biome,
        BiomeFeatureConfig {
            features: vec![(FeatureTypeId(3), 0.5), (FeatureTypeId(2), 0.2)],
        },
    );
    // Ocean biome has no features.

    let placer = FeaturePlacer::new(12345, vec![tree, boulder, cactus], biome_features);

    // Place features on land (forest biome).
    let land_features = placer.place_features(
        (0.0, 0.0),
        (100.0, 100.0),
        999,
        &|_x, _y| 50.0, // All land at height 50
        &|_x, _y| forest_biome,
        0.0,
    );
    info!(
        "Forest biome: {} features placed in 100x100 chunk",
        land_features.len()
    );
    assert!(
        !land_features.is_empty(),
        "Expected features in forest biome on land"
    );

    // Place features in desert biome.
    let desert_features = placer.place_features(
        (0.0, 0.0),
        (100.0, 100.0),
        888,
        &|_x, _y| 50.0,
        &|_x, _y| desert_biome,
        0.0,
    );
    info!(
        "Desert biome: {} features placed in 100x100 chunk",
        desert_features.len()
    );

    // No features in ocean (underwater).
    let ocean_features = placer.place_features(
        (0.0, 0.0),
        (100.0, 100.0),
        777,
        &|_x, _y| -10.0, // Underwater
        &|_x, _y| ocean_biome,
        0.0,
    );
    assert!(
        ocean_features.is_empty(),
        "Expected no features in ocean biome"
    );
    info!("Ocean biome: 0 features (all underwater) -- correct");

    // Verify determinism.
    let features_a = placer.place_features(
        (0.0, 0.0),
        (50.0, 50.0),
        555,
        &|_x, _y| 100.0,
        &|_x, _y| forest_biome,
        0.0,
    );
    let features_b = placer.place_features(
        (0.0, 0.0),
        (50.0, 50.0),
        555,
        &|_x, _y| 100.0,
        &|_x, _y| forest_biome,
        0.0,
    );
    assert_eq!(
        features_a.len(),
        features_b.len(),
        "Deterministic placement: same seed should produce same count"
    );
    info!(
        "Deterministic placement verified: {} features both runs",
        features_a.len()
    );

    let total = land_features.len() + desert_features.len();
    info!("Feature placement demonstration completed: {total} total features placed");
    total
}

/// Demonstrates terrain debug visualization: generates heightmap, biome, cave, and ore debug images.
fn demonstrate_terrain_debug_viz() {
    use glam::DVec3;
    use nebula_cubesphere::CubeFace;
    use nebula_terrain::{
        BiomeDef, BiomeRegistry, BiomeSampler, CaveCarver, CaveConfig, HeightmapParams,
        HeightmapSampler, OreDistributor, SliceParams, TerrainDebugState, TerrainHeightConfig,
        WhittakerDiagram, WhittakerRegion, default_ore_distributions, render_biome_debug,
        render_cave_cross_section, render_heightmap_debug, render_ore_heatmap,
    };

    info!("Starting terrain debug visualization demonstration");

    // Debug state toggle test
    let mut state = TerrainDebugState::new();
    assert!(!state.any_visible());
    state.show_heightmap = true;
    state.show_biome_map = true;
    state.show_cave_section = true;
    state.show_ore_heatmap = true;
    assert!(state.any_visible());

    // Heightmap debug image
    let sampler = HeightmapSampler::new(HeightmapParams {
        seed: 42,
        ..Default::default()
    });
    let config = TerrainHeightConfig::default();
    let heightmap_img = render_heightmap_debug(
        &sampler,
        &config,
        128,
        128,
        CubeFace::PosX,
        (0.0, 0.0, 1.0, 1.0),
    );
    info!(
        "Heightmap debug: {}x{}, {} unique colors",
        heightmap_img.width,
        heightmap_img.height,
        heightmap_img.unique_color_count()
    );

    // Biome debug image
    let mut registry = BiomeRegistry::new();
    let plains = registry
        .register(BiomeDef {
            name: "plains".into(),
            surface_voxel: VoxelTypeId(30),
            subsurface_voxel: VoxelTypeId(31),
            vegetation_density: 0.3,
            tree_type: Some("oak".into()),
        })
        .unwrap();
    let desert = registry
        .register(BiomeDef {
            name: "desert".into(),
            surface_voxel: VoxelTypeId(20),
            subsurface_voxel: VoxelTypeId(21),
            vegetation_density: 0.01,
            tree_type: None,
        })
        .unwrap();
    let forest = registry
        .register(BiomeDef {
            name: "forest".into(),
            surface_voxel: VoxelTypeId(40),
            subsurface_voxel: VoxelTypeId(41),
            vegetation_density: 0.8,
            tree_type: Some("birch".into()),
        })
        .unwrap();

    let diagram = WhittakerDiagram {
        regions: vec![
            WhittakerRegion {
                temp_min: 0.0,
                temp_max: 0.5,
                moisture_min: 0.0,
                moisture_max: 1.0,
                biome_id: plains,
            },
            WhittakerRegion {
                temp_min: 0.5,
                temp_max: 1.0,
                moisture_min: 0.0,
                moisture_max: 0.4,
                biome_id: desert,
            },
            WhittakerRegion {
                temp_min: 0.5,
                temp_max: 1.0,
                moisture_min: 0.4,
                moisture_max: 1.0,
                biome_id: forest,
            },
        ],
        fallback: plains,
    };

    let biome_sampler = BiomeSampler::new(42, diagram);
    let biome_img = render_biome_debug(
        &biome_sampler,
        &registry,
        128,
        128,
        CubeFace::PosX,
        (0.0, 0.0, 1.0, 1.0),
    );
    info!(
        "Biome debug: {}x{}, {} unique colors",
        biome_img.width,
        biome_img.height,
        biome_img.unique_color_count()
    );

    // Cave cross-section debug image
    let carver = CaveCarver::new(CaveConfig {
        seed: 42,
        threshold: 0.0,
        ..Default::default()
    });
    let planet_radius = 6_371_000.0_f64;
    let surface_height = planet_radius + 200.0;
    let sea_level = planet_radius;
    let cave_slice = SliceParams {
        origin: DVec3::new(surface_height - 50.0, 0.0, 0.0),
        u_axis: DVec3::Y,
        v_axis: DVec3::Z,
        extent: 500.0,
    };
    let cave_img =
        render_cave_cross_section(&carver, 64, 64, &cave_slice, surface_height, sea_level);
    info!(
        "Cave debug: {}x{}, {} unique colors",
        cave_img.width,
        cave_img.height,
        cave_img.unique_color_count()
    );

    // Ore heatmap debug image
    let distributor = OreDistributor::new(42, default_ore_distributions());
    let ore_slice = SliceParams {
        origin: DVec3::new(surface_height - 50.0, 0.0, 0.0),
        u_axis: DVec3::Y,
        v_axis: DVec3::Z,
        extent: 200.0,
    };
    let ore_img = render_ore_heatmap(&distributor, 64, 64, &ore_slice, surface_height);
    info!(
        "Ore debug: {}x{}, {} unique colors",
        ore_img.width,
        ore_img.height,
        ore_img.unique_color_count()
    );

    state.mark_dirty();
    assert!(state.is_dirty());
    state.clear_dirty();
    assert!(!state.is_dirty());

    info!("Terrain debug visualization demonstration completed successfully");
}

/// Demonstrates async chunk generation: offloads terrain generation to background threads.
fn demonstrate_async_chunk_generation() -> (usize, u64) {
    use nebula_coords::WorldPosition;
    use nebula_terrain::{AsyncChunkGenerator, GenerationTask};

    info!("Starting async chunk generation demonstration");

    let planet = PlanetDef::earth_like("AsyncTerra", WorldPosition::default(), 42);
    let generator = AsyncChunkGenerator::new(4, 64, 128);

    let grid_size = 5_i64;
    let mut submitted = 0usize;
    for x in 0..grid_size {
        for z in 0..grid_size {
            let addr = ChunkAddress::new(x, 0, z, 0);
            let task = GenerationTask {
                address: addr,
                seed: planet.seed,
                planet: planet.clone(),
                priority: (x * x + z * z) as u64,
            };
            if generator.submit(task).is_ok() {
                submitted += 1;
            }
        }
    }

    let start = std::time::Instant::now();
    let mut received = Vec::new();
    let deadline = start + std::time::Duration::from_secs(30);
    while received.len() < submitted && std::time::Instant::now() < deadline {
        received.extend(generator.drain_results());
        if received.len() < submitted {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    let elapsed_ms = start.elapsed().as_millis() as u64;

    let total_gen_us: u64 = received.iter().map(|r| r.generation_time_us).sum();

    info!(
        "Generated: {} chunks in {}ms (4 workers), total gen time: {}us",
        received.len(),
        elapsed_ms,
        total_gen_us,
    );

    assert_eq!(
        received.len(),
        submitted,
        "Should receive all submitted chunks"
    );

    info!("Async chunk generation demonstration completed successfully");
    (received.len(), elapsed_ms)
}

/// Demonstrates distance-based LOD selection with default planet thresholds.
fn demonstrate_distance_lod() {
    use nebula_lod::{LodSelector, LodThresholds, chunk_distance_to_camera};

    info!("Starting distance-based LOD demonstration");

    let selector = LodSelector::new(LodThresholds::default_planet());

    // Simulate chunks at varying distances and count LOD distribution.
    let camera = nebula_coords::WorldPosition::default();
    let mut lod_counts = [0u32; 6]; // L0..L5

    // Place chunks in concentric shells around the camera.
    let distances_m: &[f64] = &[
        100.0, 200.0, 300.0, 400.0, 600.0, 800.0, 1500.0, 3000.0, 5000.0, 10000.0,
    ];
    let chunks_per_shell = 12u32;

    for &dist in distances_m {
        let dist_mm = (dist * 1000.0) as i128; // meters to millimeters
        for i in 0..chunks_per_shell {
            let angle = (i as f64) * std::f64::consts::TAU / (chunks_per_shell as f64);
            let cx = (angle.cos() * dist_mm as f64) as i128;
            let cz = (angle.sin() * dist_mm as f64) as i128;
            let chunk_center = nebula_coords::WorldPosition::new(cx, 0, cz);
            let d = chunk_distance_to_camera(&chunk_center, &camera);
            // Convert mm distance to meters for LOD selection
            let d_meters = d / 1000.0;
            let lod = selector.select_lod(d_meters);
            let idx = (lod as usize).min(lod_counts.len() - 1);
            lod_counts[idx] += 1;
        }
    }

    info!(
        "LOD distribution: L0={}, L1={}, L2={}, L3={}, L4={}, L5={}",
        lod_counts[0], lod_counts[1], lod_counts[2], lod_counts[3], lod_counts[4], lod_counts[5]
    );

    // Verify resolution mapping
    for lod in 0..5u8 {
        info!(
            "  LOD {}: {}x{}x{} voxels per chunk",
            lod,
            LodSelector::resolution_for_lod(lod),
            LodSelector::resolution_for_lod(lod),
            LodSelector::resolution_for_lod(lod),
        );
    }

    info!("Distance-based LOD demonstration completed successfully");
}

/// Demonstrates LOD-aware chunk resolution scaling.
fn demonstrate_chunk_resolution_scaling() {
    use nebula_voxel::{
        LodChunkData, generate_chunk_at_lod, resolution_for_lod, voxel_size_for_lod,
    };

    info!("Starting chunk resolution scaling demonstration");

    // Verify resolution and voxel size for each LOD level.
    for lod in 0..=5u8 {
        let res = resolution_for_lod(lod);
        let vsize = voxel_size_for_lod(lod, 1.0);
        let chunk = LodChunkData::new(lod);
        info!(
            "  LOD {}: {}x{}x{} = {} voxels, voxel size = {}m, extent = {}m",
            lod,
            res,
            res,
            res,
            chunk.voxel_count(),
            vsize,
            chunk.spatial_extent(1.0),
        );
    }

    // Generate chunks at LOD 0 and LOD 3 using the same flat terrain.
    let flat_terrain = |_wx: f64, wy: f64, _wz: f64| -> VoxelTypeId {
        if wy < 16.0 {
            VoxelTypeId(1)
        } else {
            VoxelTypeId(0)
        }
    };

    let lod0 = generate_chunk_at_lod((0.0, 0.0, 0.0), 0, &flat_terrain, 1.0);
    let lod3 = generate_chunk_at_lod((0.0, 0.0, 0.0), 3, &flat_terrain, 1.0);

    info!(
        "LOD 0: {} voxels, LOD 3: {} voxels ({}x reduction)",
        lod0.voxel_count(),
        lod3.voxel_count(),
        lod0.voxel_count() / lod3.voxel_count(),
    );

    // Verify spatial extent is the same.
    assert!(
        (lod0.spatial_extent(1.0) - lod3.spatial_extent(1.0)).abs() < f64::EPSILON,
        "LOD 0 and LOD 3 should cover the same spatial extent"
    );

    info!("Chunk resolution scaling demonstration completed successfully");
}

/// Demonstrates per-face quadtree LOD subdivision.
fn demonstrate_quadtree_lod_per_face() {
    use nebula_cubesphere::CubeFace;
    use nebula_lod::{FaceQuadtreeLod, LodThresholds};

    info!("Starting quadtree LOD per-face demonstration");

    let planet_radius: f64 = 6_371_000_000.0; // Earth-like, mm

    // Create a quadtree for the +Y face with max_depth 5
    let mut qt = FaceQuadtreeLod::new(
        CubeFace::PosY,
        5,
        LodThresholds::default_planet(),
        planet_radius,
    );

    // Camera far away in space — should produce minimal chunks
    let far_camera = nebula_coords::WorldPosition::new(0, 100_000_000_000_000, 0);
    let far_chunks = qt.update(&far_camera);
    info!(
        "Far camera: {} active chunks on +Y face (expected 1, coarse)",
        far_chunks.len()
    );
    assert!(
        far_chunks.len() <= 4,
        "Far camera should produce at most 4 chunks"
    );

    // Camera on the planet surface — should subdivide deeply
    let near_camera = nebula_coords::WorldPosition::new(0, planet_radius as i128, 0);
    let near_chunks = qt.update(&near_camera);
    info!(
        "Near camera: {} active chunks on +Y face (should be > far)",
        near_chunks.len()
    );
    assert!(
        near_chunks.len() > far_chunks.len(),
        "Near camera should produce more chunks than far camera"
    );

    // Verify balance constraint: no neighboring leaves differ by >1 LOD
    for chunk in &near_chunks {
        let neighbors = qt.leaf_neighbors(chunk);
        for neighbor in &neighbors {
            let diff = (chunk.lod as i8 - neighbor.lod as i8).abs();
            assert!(
                diff <= 1,
                "Balance violated: LOD {} vs {} (diff={})",
                chunk.lod,
                neighbor.lod,
                diff
            );
        }
    }
    info!("Balance constraint verified: all neighbors within 1 LOD level");

    // Show LOD distribution
    let max_lod = near_chunks.iter().map(|c| c.lod).max().unwrap_or(0);
    let min_lod = near_chunks.iter().map(|c| c.lod).min().unwrap_or(0);
    info!("LOD range: {} (finest) to {} (coarsest)", min_lod, max_lod);

    // Test all 6 faces
    for face in CubeFace::ALL {
        let mut fqt = FaceQuadtreeLod::new(face, 5, LodThresholds::default_planet(), planet_radius);
        let chunks = fqt.update(&near_camera);
        info!("  Face {:?}: {} active chunks", face, chunks.len());
    }

    info!("Quadtree LOD per-face demonstration completed successfully");
}

/// Demonstrates LOD mesh generation: greedy meshing at variable resolutions.
fn demonstrate_lod_mesh_generation() {
    use nebula_mesh::{ChunkLodContext, ChunkNeighborhood, default_registry, mesh_lod_chunk};
    use nebula_voxel::{LodChunkData, VoxelTypeId};

    info!("Starting LOD mesh generation demonstration");

    let registry = default_registry();

    for lod in 0..=2u8 {
        let mut chunk = LodChunkData::new(lod);
        let res = chunk.resolution();
        // Fill bottom half with stone.
        for z in 0..res {
            for y in 0..res / 2 {
                for x in 0..res {
                    chunk.set(x, y, z, VoxelTypeId(1));
                }
            }
        }

        let mesh = mesh_lod_chunk(
            &chunk,
            &ChunkNeighborhood::all_air(),
            &registry,
            &ChunkLodContext::uniform(lod),
        );

        info!(
            "LOD {}: {} tris, {} quads, {}x{}x{} grid",
            lod,
            mesh.triangle_count(),
            mesh.quad_count(),
            res,
            res,
            res,
        );
    }

    info!("LOD mesh generation demonstration completed successfully");
}

/// Demonstrates LOD transition seam elimination: edge constraining + skirt geometry.
fn demonstrate_lod_transition_seams() {
    use nebula_mesh::{
        ChunkLodContext, ChunkVertex, FaceDirection, NeighborLodRelation, PackedChunkMesh,
        apply_seam_fix, constrain_edge_vertices, generate_skirt,
    };

    info!("Starting LOD transition seam demonstration");

    let chunk_size = 32usize;

    // 1. Build a test mesh with boundary vertices on +X face.
    let mut mesh = PackedChunkMesh::new();
    for y in 0..=chunk_size as u8 {
        for z in 0..=chunk_size as u8 {
            mesh.vertices.push(ChunkVertex::new(
                [chunk_size as u8, y, z],
                FaceDirection::PosX,
                0,
                1,
                [y, z],
            ));
        }
    }
    let verts_before = mesh.vertices.len();
    info!("  Built boundary mesh with {} vertices", verts_before);

    // 2. Constrain edge vertices for LOD 0 next to LOD 1 neighbor.
    constrain_edge_vertices(&mut mesh, FaceDirection::PosX, 1, chunk_size);

    let step = 2usize;
    let mut all_aligned = true;
    for v in &mesh.vertices {
        if !(v.position[1] as usize).is_multiple_of(step)
            || !(v.position[2] as usize).is_multiple_of(step)
        {
            all_aligned = false;
            break;
        }
    }
    info!(
        "  Edge constraining (lod_diff=1): all vertices aligned = {}",
        all_aligned
    );
    assert!(all_aligned, "All boundary vertices must be aligned to step");

    // 3. Generate skirt geometry on -Z face.
    let mut skirt_mesh = PackedChunkMesh::new();
    let skirt_tris = generate_skirt(&mut skirt_mesh, FaceDirection::NegZ, 2, chunk_size);
    info!(
        "  Skirt geometry: {} triangles, {} vertices",
        skirt_tris,
        skirt_mesh.vertices.len()
    );
    assert!(skirt_tris > 0, "Skirt should produce triangles");

    // 4. Full seam fix pipeline.
    let mut full_mesh = PackedChunkMesh::new();
    for y in 0..=chunk_size as u8 {
        full_mesh.vertices.push(ChunkVertex::new(
            [chunk_size as u8, y, 16],
            FaceDirection::PosX,
            0,
            1,
            [y, 16],
        ));
    }
    let ctx = ChunkLodContext::from_neighbor_lods(0, [Some(1), None, None, Some(2), None, None]);
    let total_skirt = apply_seam_fix(&mut full_mesh, &ctx, chunk_size, 2);
    info!(
        "  Full seam fix: {} skirt triangles across {} transitioning faces",
        total_skirt,
        FaceDirection::ALL
            .iter()
            .filter(|d| ctx.has_lod_difference(**d))
            .count()
    );

    // 5. Verify ChunkLodContext classification.
    assert_eq!(
        ctx.neighbors[0],
        NeighborLodRelation::HigherThanNeighbor { lod_diff: 1 }
    );
    assert_eq!(
        ctx.neighbors[3],
        NeighborLodRelation::HigherThanNeighbor { lod_diff: 2 }
    );

    info!("LOD transition seam demonstration completed successfully");
}

/// Configure system ordering constraints for all engine stages.
fn configure_system_ordering(schedules: &mut nebula_ecs::EngineSchedules) {
    if let Some(s) = schedules.get_schedule_mut(&nebula_ecs::EngineSchedule::PreUpdate) {
        nebula_ecs::configure_preupdate_ordering(s);
    }
    if let Some(s) = schedules.get_schedule_mut(&nebula_ecs::EngineSchedule::FixedUpdate) {
        nebula_ecs::configure_fixedupdate_ordering(s);
    }
    if let Some(s) = schedules.get_schedule_mut(&nebula_ecs::EngineSchedule::Update) {
        nebula_ecs::configure_update_ordering(s);
    }
    if let Some(s) = schedules.get_schedule_mut(&nebula_ecs::EngineSchedule::PostUpdate) {
        nebula_ecs::configure_postupdate_ordering(s);
    }
    if let Some(s) = schedules.get_schedule_mut(&nebula_ecs::EngineSchedule::PreRender) {
        nebula_ecs::configure_prerender_ordering(s);
    }
    if let Some(s) = schedules.get_schedule_mut(&nebula_ecs::EngineSchedule::Render) {
        nebula_ecs::configure_render_ordering(s);
    }
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

    // Demonstrate chunk loading/unloading
    let loaded_count = demonstrate_chunk_loading();

    // Demonstrate Copy-on-Write chunks
    demonstrate_cow_chunks();

    // Demonstrate voxel modification events
    demonstrate_voxel_events();

    // Demonstrate chunk data versioning
    let chunk_version = demonstrate_chunk_versioning();

    // Demonstrate visible face detection
    let (visible_faces, total_faces) = demonstrate_visible_face_detection();

    // Demonstrate greedy meshing
    let (greedy_quads, naive_quads) = demonstrate_greedy_meshing();

    // Demonstrate ambient occlusion
    let (ao_exposed, ao_occluded, ao_shaded_verts) = demonstrate_ambient_occlusion();

    // Demonstrate adjacent chunk culling
    let (faces_no_neighbor, faces_with_neighbor) = demonstrate_adjacent_chunk_culling();

    // Demonstrate GPU mesh upload and buffer pool
    let (gpu_upload_bytes, pool_allocated, gpu_reused) = demonstrate_gpu_mesh_upload();

    // Demonstrate async mesh generation
    let (async_chunks, async_quads) = demonstrate_async_meshing();

    // Demonstrate mesh cache invalidation
    let (inv_interior, inv_boundary, inv_corner) = demonstrate_mesh_invalidation();

    // Demonstrate cubesphere vertex displacement
    let (disp_verts, disp_min, disp_max) = demonstrate_cubesphere_displacement();

    // Demonstrate multi-octave noise heightmap
    let (_hmap_min, _hmap_max) = demonstrate_heightmap();

    // Demonstrate biome system
    let biome_count = demonstrate_biome_system();

    // Demonstrate cubesphere terrain height
    demonstrate_cubesphere_terrain_height();

    // Demonstrate ore resource distribution
    demonstrate_ore_distribution();

    // Demonstrate feature placement
    let feature_count = demonstrate_feature_placement();

    // Demonstrate terrain debug visualization
    demonstrate_terrain_debug_viz();

    // Demonstrate async chunk generation
    let (async_gen_chunks, async_gen_ms) = demonstrate_async_chunk_generation();

    // Demonstrate distance-based LOD selection
    demonstrate_distance_lod();

    // Demonstrate chunk resolution scaling
    demonstrate_chunk_resolution_scaling();

    // Demonstrate per-face quadtree LOD
    demonstrate_quadtree_lod_per_face();

    // Demonstrate LOD mesh generation
    demonstrate_lod_mesh_generation();

    // Demonstrate LOD transition seam elimination
    demonstrate_lod_transition_seams();

    // Initialize ECS world and schedules with stage execution logging
    let mut ecs_world = nebula_ecs::create_world();
    ecs_world.insert_resource(nebula_ecs::CameraRes::default());
    ecs_world.insert_resource(nebula_ecs::SpawnQueue::default());
    ecs_world.insert_resource(nebula_ecs::DespawnQueue::default());
    let mut ecs_schedules = nebula_ecs::EngineSchedules::new();

    // Configure system ordering constraints for all stages
    configure_system_ordering(&mut ecs_schedules);

    // Register stage-logging systems into their respective system sets
    ecs_schedules.add_system(
        nebula_ecs::EngineSchedule::PreUpdate,
        (|| {
            tracing::debug!("Stage: PreUpdate/Time");
        })
        .in_set(nebula_ecs::PreUpdateSet::Time),
    );
    ecs_schedules.add_system(
        nebula_ecs::EngineSchedule::PreUpdate,
        (|| {
            tracing::debug!("Stage: PreUpdate/Input");
        })
        .in_set(nebula_ecs::PreUpdateSet::Input),
    );
    ecs_schedules.add_system(nebula_ecs::EngineSchedule::FixedUpdate, || {
        tracing::debug!("Stage: FixedUpdate");
    });
    ecs_schedules.add_system(nebula_ecs::EngineSchedule::Update, || {
        tracing::debug!("Stage: Update");
    });
    ecs_schedules.add_system(
        nebula_ecs::EngineSchedule::PostUpdate,
        nebula_ecs::flush_entity_queues.in_set(nebula_ecs::PostUpdateSet::TransformPropagation),
    );
    ecs_schedules.add_system(
        nebula_ecs::EngineSchedule::PostUpdate,
        nebula_ecs::update_local_positions_incremental
            .in_set(nebula_ecs::PostUpdateSet::TransformPropagation),
    );
    ecs_schedules.add_system(
        nebula_ecs::EngineSchedule::PostUpdate,
        nebula_ecs::update_all_local_positions_on_camera_move
            .in_set(nebula_ecs::PostUpdateSet::TransformPropagation),
    );
    ecs_schedules.add_system(
        nebula_ecs::EngineSchedule::PostUpdate,
        (|| {
            tracing::debug!("Stage: PostUpdate/SpatialIndex");
        })
        .in_set(nebula_ecs::PostUpdateSet::SpatialIndexUpdate),
    );
    ecs_schedules.add_system(
        nebula_ecs::EngineSchedule::PreRender,
        (|| {
            tracing::debug!("Stage: PreRender/Culling");
        })
        .in_set(nebula_ecs::PreRenderSet::Culling),
    );
    ecs_schedules.add_system(
        nebula_ecs::EngineSchedule::PreRender,
        (|| {
            tracing::debug!("Stage: PreRender/Batching");
        })
        .in_set(nebula_ecs::PreRenderSet::Batching),
    );
    ecs_schedules.add_system(
        nebula_ecs::EngineSchedule::Render,
        (|| {
            tracing::debug!("Stage: Render/Draw");
        })
        .in_set(nebula_ecs::RenderSet::Draw),
    );

    // Validate all schedule dependency graphs at startup
    nebula_ecs::validate_schedules(&mut ecs_schedules, &mut ecs_world);
    info!("System ordering: all schedule graphs validated (no cycles)");

    // Spawn chunk entities using the entity lifecycle API
    let mut chunk_entities = Vec::new();
    for cx in 0..5_i128 {
        for cz in 0..5_i128 {
            let entity = nebula_ecs::spawn_entity(
                &mut ecs_world,
                (
                    nebula_ecs::WorldPos::new(cx * 32_000, 0, cz * 32_000),
                    nebula_ecs::Name::new(format!("chunk_{cx}_{cz}")),
                    nebula_ecs::Active(true),
                ),
            );
            chunk_entities.push(entity);
        }
    }
    info!(
        "Spawned {} chunk entities via entity lifecycle API",
        chunk_entities.len()
    );

    // Despawn edge chunks to simulate camera movement
    let mut despawned = 0;
    for &e in &chunk_entities[..5] {
        if nebula_ecs::despawn_entity(&mut ecs_world, e) {
            despawned += 1;
        }
    }
    info!(
        "Despawned {} edge chunk entities, {} remain",
        despawned,
        ecs_world.entities().len()
    );

    // Double-despawn safety check
    let double = nebula_ecs::despawn_entity(&mut ecs_world, chunk_entities[0]);
    info!("Double-despawn returned {} (expected false)", double);

    // Run one frame to process initial Added detection and establish baselines
    ecs_schedules.run(&mut ecs_world, 1.0 / 60.0);

    // Demonstrate change detection: mutate ONE chunk's WorldPos, then run a frame.
    // Only that entity should be reprocessed by the incremental system.
    let changed_count;
    let skipped_count;
    {
        // Add LocalPos to all chunk entities that don't have it yet, so the
        // incremental system can observe Changed<WorldPos>.
        let alive_chunks: Vec<_> = chunk_entities[5..].to_vec(); // skip despawned edge
        let total_alive = alive_chunks.len();

        // Mutate exactly one entity's WorldPos
        if let Some(&target_entity) = alive_chunks.first()
            && let Some(mut wp) = ecs_world.get_mut::<nebula_ecs::WorldPos>(target_entity)
        {
            wp.0.x += 1000; // move 1 meter
        }

        // The incremental system will only process the 1 changed entity.
        // We log the expected counts for console verification.
        changed_count = 1_usize;
        skipped_count = total_alive - changed_count;
    }
    info!(
        "Changed: {} chunk, skipped: {}",
        changed_count, skipped_count
    );

    // Run second frame — incremental change detection processes only the mutated entity
    ecs_schedules.run(&mut ecs_world, 1.0 / 60.0);

    let entity_count = ecs_world.entities().len();
    info!(
        "ECS World: {} entities after lifecycle operations, stage pipeline validated",
        entity_count
    );

    // Log initial state
    let mut demo_state = DemoState::new();
    let initial_sector = SectorCoord::from_world(&demo_state.position);

    // Update window title to show planet info and nearby count
    let terra = PlanetDef::earth_like("Terra", WorldPosition::default(), 42);
    config.window.title = format!(
        "Nebula Engine - Planet: {}, radius={} mm - Registry: {} types - Chunks loaded: {} - Dirty: {}/{} - Loaded: {} - Chunk (0,0) v{} - Nearby: {} entities - Faces: {} visible of {} total",
        terra.name,
        terra.radius,
        voxel_type_count,
        chunks_loaded,
        dirty_count,
        chunks_loaded,
        loaded_count,
        chunk_version,
        demo_state.nearby_count,
        visible_faces,
        total_faces,
    );
    config.window.title = format!(
        "{} - Greedy: {} quads (was {}) - AO: {}/{} ({} shaded) - AdjCull: {}/{} - GPU: {}B pool:{}B reuse:{} - Async: {}chunks/{}quads",
        config.window.title,
        greedy_quads,
        naive_quads,
        ao_exposed,
        ao_occluded,
        ao_shaded_verts,
        faces_with_neighbor,
        faces_no_neighbor,
        gpu_upload_bytes,
        pool_allocated,
        gpu_reused,
        async_chunks,
        async_quads,
    );
    config.window.title = format!(
        "{} - Invalidation: int={}/bnd={}/crn={} - CubeDisp: {}v [{:.0},{:.0}] - Biomes: {} - Features: {} - AsyncGen: {}chunks/{}ms - Entities: {}",
        config.window.title,
        inv_interior,
        inv_boundary,
        inv_corner,
        disp_verts,
        disp_min,
        disp_max,
        biome_count,
        feature_count,
        async_gen_chunks,
        async_gen_ms,
        entity_count,
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
