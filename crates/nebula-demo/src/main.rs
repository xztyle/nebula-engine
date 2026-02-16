//! Demo binary that opens a Nebula Engine window with GPU-cleared background.
//!
//! Configuration is loaded from `config.ron` and can be overridden via CLI flags.
//! Run with `cargo run -p nebula-demo` to see the window.
//! Run with `cargo run -p nebula-demo -- --width 1920 --height 1080` to override size.

use clap::Parser;
use nebula_config::{CliArgs, Config};
use nebula_coords::{EntityId, SectorCoord, SpatialEntity, SpatialHashMap, WorldPosition};
use nebula_cubesphere::{
    ChunkAddress, CubeCorner, CubeFace, FaceCoord, FaceDirection, FaceQuadtree, LodNeighbor,
    SameFaceNeighbor, corner_lod_valid, direction_to_face, face_coord_to_sphere_everitt,
    sphere_to_face_coord_everitt,
};
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
fn demonstrate_cubesphere_projection() {
    info!("Starting cubesphere projection demonstration");

    let subdivisions = 8;
    let mut total_points = 0;
    let mut max_deviation: f64 = 0.0;

    for face in CubeFace::ALL {
        for u_step in 0..=subdivisions {
            for v_step in 0..=subdivisions {
                let u = u_step as f64 / subdivisions as f64;
                let v = v_step as f64 / subdivisions as f64;
                let fc = FaceCoord::new(face, u, v);
                let sphere_pt = face_coord_to_sphere_everitt(&fc);
                let deviation = (sphere_pt.length() - 1.0).abs();
                if deviation > max_deviation {
                    max_deviation = deviation;
                }
                total_points += 1;
            }
        }
    }

    info!(
        "Projected {} points onto unit sphere (max deviation: {:.2e})",
        total_points, max_deviation
    );
    info!("Cubesphere projection demonstration completed successfully");
}

/// Demonstrates sphere-to-cube inverse projection by picking random sphere
/// points, mapping them back to face coordinates, and verifying the roundtrip.
fn demonstrate_sphere_to_cube_inverse() {
    info!("Starting sphere-to-cube inverse demonstration");

    let mut rng = Xoshiro256StarStar::seed_from_u64(77);
    let sample_count = 200;
    let mut max_error: f64 = 0.0;
    let mut face_counts = [0u32; 6];

    for _ in 0..sample_count {
        // Generate a random point on the unit sphere
        let theta: f64 = rng.gen_range(0.0..std::f64::consts::TAU);
        let phi: f64 = rng.gen_range(-1.0_f64..1.0).acos();
        let dir = glam::DVec3::new(phi.sin() * theta.cos(), phi.sin() * theta.sin(), phi.cos());
        let dir = dir.normalize();

        // Determine face
        let face = direction_to_face(dir);
        face_counts[face as usize] += 1;

        // Roundtrip through Everitt inverse
        // Pick a random face coord, project to sphere, then invert
        let u: f64 = rng.gen_range(0.01..0.99);
        let v: f64 = rng.gen_range(0.01..0.99);
        let original = FaceCoord::new(face, u, v);
        let sphere_pt = face_coord_to_sphere_everitt(&original);
        let recovered = sphere_to_face_coord_everitt(sphere_pt);

        let err_u = (recovered.u - original.u).abs();
        let err_v = (recovered.v - original.v).abs();
        let err = err_u.max(err_v);
        if err > max_error {
            max_error = err;
        }
    }

    info!(
        "Inverse projection: {} roundtrips, max error: {:.2e}",
        sample_count, max_error
    );
    info!(
        "Face distribution: +X={} -X={} +Y={} -Y={} +Z={} -Z={}",
        face_counts[0],
        face_counts[1],
        face_counts[2],
        face_counts[3],
        face_counts[4],
        face_counts[5]
    );
    info!("Sphere-to-cube inverse demonstration completed successfully");
}

/// Demonstrates same-face neighbor finding on the cubesphere.
fn demonstrate_neighbor_finding() {
    info!("Starting same-face neighbor finding demonstration");

    // Same-LOD neighbor finding: a center chunk should have 4 neighbors
    let center = ChunkAddress::new(CubeFace::PosX, 10, 50, 50);
    let mut same_face_count = 0;
    for dir in FaceDirection::ALL {
        if let SameFaceNeighbor::Same(n) = center.same_face_neighbor(dir) {
            same_face_count += 1;
            info!("  {:?} neighbor of {}: {} (lod={})", dir, center, n, n.lod);
        }
    }
    info!(
        "Center chunk at {} has {} same-face neighbors",
        center, same_face_count
    );

    // Edge chunk: should have 3 same-face neighbors
    let edge = ChunkAddress::new(CubeFace::PosX, 10, 0, 50);
    let mut edge_count = 0;
    for dir in FaceDirection::ALL {
        if matches!(edge.same_face_neighbor(dir), SameFaceNeighbor::Same(_)) {
            edge_count += 1;
        }
    }
    info!(
        "Edge chunk at {} has {} same-face neighbors",
        edge, edge_count
    );

    // LOD-aware neighbor finding with a quadtree
    let mut tree = FaceQuadtree::new(CubeFace::PosX);
    tree.root.subdivide();

    // Subdivide bottom-right child to create LOD mismatch
    if let nebula_cubesphere::QuadNode::Branch { children, .. } = &mut tree.root {
        children[1].subdivide();
    }

    let leaves = tree.root.all_leaves();
    let coarse_leaf = leaves
        .iter()
        .find(|a| a.lod == ChunkAddress::MAX_LOD - 1 && a.x == 0 && a.y == 0)
        .unwrap();

    let result = tree.find_neighbor(coarse_leaf, FaceDirection::East);
    match &result {
        LodNeighbor::Single(n) => info!("LOD-aware: single neighbor at {n}"),
        LodNeighbor::Multiple(ns) => {
            info!("LOD-aware: {} finer neighbors along shared edge", ns.len());
        }
        LodNeighbor::OffFace => info!("LOD-aware: neighbor is off-face"),
    }

    info!("Same-face neighbor finding demonstration completed successfully");
}

/// Demonstrates cross-face corner neighbor finding at cube corners.
fn demonstrate_corner_neighbors() {
    info!("Starting cross-face corner neighbor demonstration");

    let mut corners_found = 0;
    let lod = 10;
    let grid = ChunkAddress::grid_size(lod);

    // Check all 4 corners of PosX face
    let corner_coords = [(0, 0), (grid - 1, 0), (0, grid - 1), (grid - 1, grid - 1)];
    for (x, y) in corner_coords {
        let addr = ChunkAddress::new(CubeFace::PosX, lod, x, y);
        if let Some(neighbors) = addr.corner_neighbors() {
            corners_found += 1;
            info!(
                "  Corner {:?}: {} neighbors on {:?} and {:?}",
                neighbors.corner, addr, neighbors.neighbor_a.face, neighbors.neighbor_b.face,
            );
        }
    }
    info!("Found {corners_found} corner adjacencies on PosX face");

    // Verify all 8 cube corners have consistent 3-face meetings
    for corner in CubeCorner::ALL {
        let faces = corner.faces();
        let pos = corner.position();
        info!(
            "  Corner {:?} at ({:.0},{:.0},{:.0}): faces {:?}, {:?}, {:?}",
            corner, pos.x, pos.y, pos.z, faces[0], faces[1], faces[2]
        );
    }

    // Demonstrate LOD validation at corners
    let a = ChunkAddress::new(CubeFace::PosX, 10, 0, 0);
    let b = ChunkAddress::new(CubeFace::PosY, 10, 0, 0);
    let c = ChunkAddress::new(CubeFace::PosZ, 10, 0, 0);
    let same_lod_valid = corner_lod_valid(&a, &b, &c);

    let c_bad = ChunkAddress::new(CubeFace::PosZ, 12, 0, 0);
    let big_gap_valid = corner_lod_valid(&a, &b, &c_bad);

    info!(
        "LOD validation: same LOD={}, gap of 2={}",
        same_lod_valid, big_gap_valid
    );

    info!("Cross-face corner neighbor demonstration completed successfully");
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
    demonstrate_cubesphere_projection();

    // Demonstrate sphere-to-cube inverse projection
    demonstrate_sphere_to_cube_inverse();

    // Demonstrate same-face neighbor finding
    demonstrate_neighbor_finding();

    // Demonstrate cross-face corner neighbors
    demonstrate_corner_neighbors();

    // Log initial state
    let mut demo_state = DemoState::new();
    let initial_sector = SectorCoord::from_world(&demo_state.position);

    // Update window title to show nearby count and triangle
    config.window.title = format!(
        "Nebula Engine [Triangle Demo] - Nearby: {} entities",
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
