//! Cubesphere-related demonstration functions.

use nebula_coords::WorldPosition;
use nebula_cubesphere::{
    ChunkAddress, CubeCorner, CubeFace, FaceCoord, FaceDirection, FaceQuadtree, LodNeighbor,
    PlanetDef, PlanetRegistry, SameFaceNeighbor, corner_lod_valid, direction_to_face,
    face_coord_to_sphere_everitt, face_uv_to_world_position, sphere_to_face_coord_everitt,
    world_position_to_face_uv,
};
use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256StarStar;
use tracing::info;

/// Demonstrates cubesphere projection by projecting grid points onto a sphere.
pub(crate) fn demonstrate_cubesphere_projection() {
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
pub(crate) fn demonstrate_sphere_to_cube_inverse() {
    info!("Starting sphere-to-cube inverse demonstration");

    let mut rng = Xoshiro256StarStar::seed_from_u64(77);
    let sample_count = 200;
    let mut max_error: f64 = 0.0;
    let mut face_counts = [0u32; 6];

    for _ in 0..sample_count {
        let theta: f64 = rng.gen_range(0.0..std::f64::consts::TAU);
        let phi: f64 = rng.gen_range(-1.0_f64..1.0).acos();
        let dir = glam::DVec3::new(phi.sin() * theta.cos(), phi.sin() * theta.sin(), phi.cos());
        let dir = dir.normalize();

        let face = direction_to_face(dir);
        face_counts[face as usize] += 1;

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
pub(crate) fn demonstrate_neighbor_finding() {
    info!("Starting same-face neighbor finding demonstration");

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

    let mut tree = FaceQuadtree::new(CubeFace::PosX);
    tree.root.subdivide();

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
pub(crate) fn demonstrate_corner_neighbors() {
    info!("Starting cross-face corner neighbor demonstration");

    let mut corners_found = 0;
    let lod = 10;
    let grid = ChunkAddress::grid_size(lod);

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

    for corner in CubeCorner::ALL {
        let faces = corner.faces();
        let pos = corner.position();
        info!(
            "  Corner {:?} at ({:.0},{:.0},{:.0}): faces {:?}, {:?}, {:?}",
            corner, pos.x, pos.y, pos.z, faces[0], faces[1], faces[2]
        );
    }

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

/// Demonstrates face-UV-to-world-position conversion.
pub(crate) fn demonstrate_face_uv_to_world() {
    info!("Starting face UV to world position demonstration");

    let earth_radius: i128 = 6_371_000_000;
    let planet_center = WorldPosition::new(0, 0, 0);

    let fc = FaceCoord::new(CubeFace::PosX, 0.5, 0.5);
    let world_pos = face_uv_to_world_position(&fc, earth_radius, 0, &planet_center);
    info!("Clicked: {world_pos}");

    let (fc_back, height_back) =
        world_position_to_face_uv(&world_pos, earth_radius, &planet_center);
    info!(
        "Roundtrip: face={:?} u={:.6} v={:.6} height={} mm",
        fc_back.face, fc_back.u, fc_back.v, height_back
    );

    let mountain_height: i64 = 1_000_000;
    let fc_mountain = FaceCoord::new(CubeFace::PosY, 0.3, 0.7);
    let pos_mountain =
        face_uv_to_world_position(&fc_mountain, earth_radius, mountain_height, &planet_center);
    info!("Mountain top: {pos_mountain}");

    for face in CubeFace::ALL {
        let fc_face = FaceCoord::new(face, 0.5, 0.5);
        let pos = face_uv_to_world_position(&fc_face, earth_radius, 0, &planet_center);
        let dist =
            ((pos.x as f64).powi(2) + (pos.y as f64).powi(2) + (pos.z as f64).powi(2)).sqrt();
        info!(
            "  {face:?} center: {pos} (dist from center: {:.0} mm)",
            dist
        );
    }

    info!("Face UV to world position demonstration completed successfully");
}

/// Demonstrates planet definition and registry functionality.
pub(crate) fn demonstrate_planet_definition() {
    info!("Starting planet definition demonstration");

    let mut registry = PlanetRegistry::new();

    let terra = PlanetDef::earth_like("Terra", WorldPosition::new(0, 0, 0), 42);
    let luna = PlanetDef::moon_like("Luna", WorldPosition::new(384_400_000_000, 0, 0), 43);
    let mars = PlanetDef::mars_like("Mars", WorldPosition::new(225_000_000_000_000, 0, 0), 44);

    info!(
        "Terra: radius={} mm, circumference={:.0} mm, surface_area={:.2e} mm²",
        terra.radius,
        terra.circumference_mm(),
        terra.surface_area_mm2(),
    );

    registry.register(terra).unwrap();
    registry.register(luna).unwrap();
    registry.register(mars).unwrap();

    info!("Registered {} planets", registry.len());

    for planet in registry.iter() {
        let origin_inside = planet.contains(&WorldPosition::default());
        info!(
            "  {} (seed={}): origin inside = {}",
            planet.name, planet.seed, origin_inside
        );
    }

    let too_close = PlanetDef::earth_like("TooClose", WorldPosition::new(1_000_000_000, 0, 0), 99);
    let result = registry.register(too_close);
    assert!(result.is_err(), "Overlapping planet should be rejected");
    info!("Overlap detection working: {}", result.unwrap_err());

    info!("Planet definition demonstration completed successfully");
}
