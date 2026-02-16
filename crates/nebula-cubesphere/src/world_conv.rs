//! Convert between face-UV coordinates and world positions.
//!
//! This module bridges the cubesphere abstraction and the engine's universal
//! coordinate system ([`WorldPosition`] with i128 millimeter precision).

use glam::DVec3;
use nebula_math::WorldPosition;

use crate::inverse::sphere_to_face_coord_everitt;
use crate::projection::face_coord_to_sphere_everitt;
use crate::{ChunkAddress, FaceCoord};

/// Convert a face coordinate + planet parameters to a [`WorldPosition`].
///
/// - `fc`: the face-local coordinate (face, u, v)
/// - `planet_radius`: planet radius in mm (i128)
/// - `terrain_height`: height above the sphere surface in mm (i64, signed for oceans)
/// - `planet_center`: the planet's center position in world space
///
/// Returns the [`WorldPosition`] of the point on the planet surface.
#[must_use]
pub fn face_uv_to_world_position(
    fc: &FaceCoord,
    planet_radius: i128,
    terrain_height: i64,
    planet_center: &WorldPosition,
) -> WorldPosition {
    // Step 1: FaceCoord -> unit sphere point
    let unit_dir: DVec3 = face_coord_to_sphere_everitt(fc);

    // Step 2: Scale by (radius + height) to get planet-relative position
    let total_radius = planet_radius as f64 + terrain_height as f64;
    let scaled: DVec3 = unit_dir * total_radius;

    // Step 3: Convert to i128 (round to nearest mm)
    let px = scaled.x.round() as i128;
    let py = scaled.y.round() as i128;
    let pz = scaled.z.round() as i128;

    // Step 4: Offset by planet center
    WorldPosition::new(
        planet_center.x + px,
        planet_center.y + py,
        planet_center.z + pz,
    )
}

/// Convert a [`WorldPosition`] to a [`FaceCoord`] + height relative to a planet.
///
/// Returns `(FaceCoord, terrain_height_mm)`.
#[must_use]
pub fn world_position_to_face_uv(
    world_pos: &WorldPosition,
    planet_radius: i128,
    planet_center: &WorldPosition,
) -> (FaceCoord, i64) {
    // Step 1: Compute planet-relative position
    let dx = (world_pos.x - planet_center.x) as f64;
    let dy = (world_pos.y - planet_center.y) as f64;
    let dz = (world_pos.z - planet_center.z) as f64;

    let dir = DVec3::new(dx, dy, dz);
    let distance = dir.length();

    // Step 2: Compute terrain height
    let terrain_height = (distance - planet_radius as f64).round() as i64;

    // Step 3: Convert direction to FaceCoord
    let fc = sphere_to_face_coord_everitt(dir.normalize());

    (fc, terrain_height)
}

/// Convert a grid of face-local positions to world positions for mesh generation.
///
/// `heights` is a 2D slice of terrain heights indexed by `[ui][vi]`.
/// Returns a `Vec` of [`WorldPosition`]s in row-major order (v outer, u inner).
#[must_use]
pub fn face_grid_to_world_positions(
    addr: &ChunkAddress,
    planet_radius: i128,
    planet_center: &WorldPosition,
    grid_resolution: u32,
    heights: &[Vec<i64>],
) -> Vec<WorldPosition> {
    let (u_min, v_min, u_max, v_max) = addr.uv_bounds();
    let capacity = ((grid_resolution + 1) * (grid_resolution + 1)) as usize;
    let mut positions = Vec::with_capacity(capacity);

    for vi in 0..=grid_resolution {
        for ui in 0..=grid_resolution {
            let u = u_min + (u_max - u_min) * (ui as f64 / grid_resolution as f64);
            let v = v_min + (v_max - v_min) * (vi as f64 / grid_resolution as f64);
            let fc = FaceCoord::new(addr.face, u, v);
            let h = heights[ui as usize][vi as usize];
            positions.push(face_uv_to_world_position(
                &fc,
                planet_radius,
                h,
                planet_center,
            ));
        }
    }

    positions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CubeFace;

    const EARTH_RADIUS: i128 = 6_371_000_000; // mm
    const ORIGIN: WorldPosition = WorldPosition { x: 0, y: 0, z: 0 };

    #[test]
    fn test_face_center_at_radius_maps_to_correct_axis() {
        // PosX face center at height=0 should be at approximately (radius, 0, 0)
        let fc = FaceCoord::new(CubeFace::PosX, 0.5, 0.5);
        let pos = face_uv_to_world_position(&fc, EARTH_RADIUS, 0, &ORIGIN);
        assert!(
            (pos.x - EARTH_RADIUS).abs() < 2,
            "PosX face center x: expected ~{EARTH_RADIUS}, got {}",
            pos.x
        );
        assert!(
            pos.y.abs() < 2,
            "PosX face center y should be ~0, got {}",
            pos.y
        );
        assert!(
            pos.z.abs() < 2,
            "PosX face center z should be ~0, got {}",
            pos.z
        );

        // PosY face center should be at (0, radius, 0)
        let fc_y = FaceCoord::new(CubeFace::PosY, 0.5, 0.5);
        let pos_y = face_uv_to_world_position(&fc_y, EARTH_RADIUS, 0, &ORIGIN);
        assert!(pos_y.x.abs() < 2);
        assert!((pos_y.y - EARTH_RADIUS).abs() < 2);
        assert!(pos_y.z.abs() < 2);

        // NegZ face center should be at (0, 0, -radius)
        let fc_nz = FaceCoord::new(CubeFace::NegZ, 0.5, 0.5);
        let pos_nz = face_uv_to_world_position(&fc_nz, EARTH_RADIUS, 0, &ORIGIN);
        assert!(pos_nz.x.abs() < 2);
        assert!(pos_nz.y.abs() < 2);
        assert!((pos_nz.z + EARTH_RADIUS).abs() < 2);
    }

    #[test]
    fn test_uv_corners_map_to_expected_directions() {
        let fc_00 = FaceCoord::new(CubeFace::PosX, 0.0, 0.0);
        let fc_11 = FaceCoord::new(CubeFace::PosX, 1.0, 1.0);
        let pos_00 = face_uv_to_world_position(&fc_00, EARTH_RADIUS, 0, &ORIGIN);
        let pos_11 = face_uv_to_world_position(&fc_11, EARTH_RADIUS, 0, &ORIGIN);

        let d_00 =
            ((pos_00.x as f64).powi(2) + (pos_00.y as f64).powi(2) + (pos_00.z as f64).powi(2))
                .sqrt();
        let d_11 =
            ((pos_11.x as f64).powi(2) + (pos_11.y as f64).powi(2) + (pos_11.z as f64).powi(2))
                .sqrt();
        assert!(
            (d_00 - EARTH_RADIUS as f64).abs() < 10.0,
            "Corner (0,0) not on sphere: {d_00}"
        );
        assert!(
            (d_11 - EARTH_RADIUS as f64).abs() < 10.0,
            "Corner (1,1) not on sphere: {d_11}"
        );
    }

    #[test]
    fn test_height_zero_puts_point_on_sphere_surface() {
        for face in CubeFace::ALL {
            let fc = FaceCoord::new(face, 0.3, 0.7);
            let pos = face_uv_to_world_position(&fc, EARTH_RADIUS, 0, &ORIGIN);
            let dist =
                ((pos.x as f64).powi(2) + (pos.y as f64).powi(2) + (pos.z as f64).powi(2)).sqrt();
            assert!(
                (dist - EARTH_RADIUS as f64).abs() < 10.0,
                "Height=0 point not on sphere for {face:?}: dist={dist}"
            );
        }
    }

    #[test]
    fn test_large_planet_radius_uses_full_i128_range() {
        let huge_radius: i128 = 1_000_000_000_000_000;
        let fc = FaceCoord::new(CubeFace::PosX, 0.5, 0.5);
        let pos = face_uv_to_world_position(&fc, huge_radius, 0, &ORIGIN);
        assert!(
            (pos.x - huge_radius).abs() < 1_000,
            "Large radius: expected x ~{huge_radius}, got {}",
            pos.x
        );
    }

    #[test]
    fn test_planet_center_offset() {
        let center = WorldPosition::new(
            1_000_000_000_000_000,
            -500_000_000_000_000,
            2_000_000_000_000_000,
        );
        let fc = FaceCoord::new(CubeFace::PosX, 0.5, 0.5);
        let pos = face_uv_to_world_position(&fc, EARTH_RADIUS, 0, &center);
        assert!((pos.x - center.x - EARTH_RADIUS).abs() < 2);
        assert!((pos.y - center.y).abs() < 2);
        assert!((pos.z - center.z).abs() < 2);
    }

    #[test]
    fn test_roundtrip_world_to_face_uv_to_world() {
        let fc_orig = FaceCoord::new(CubeFace::PosZ, 0.4, 0.6);
        let height: i64 = 5_000; // 5 meters
        let center = WorldPosition::new(0, 0, 0);

        let world_pos = face_uv_to_world_position(&fc_orig, EARTH_RADIUS, height, &center);
        let (fc_back, height_back) = world_position_to_face_uv(&world_pos, EARTH_RADIUS, &center);

        assert_eq!(fc_back.face, fc_orig.face);
        assert!(
            (fc_back.u - fc_orig.u).abs() < 1e-6,
            "u roundtrip: {} -> {}",
            fc_orig.u,
            fc_back.u
        );
        assert!(
            (fc_back.v - fc_orig.v).abs() < 1e-6,
            "v roundtrip: {} -> {}",
            fc_orig.v,
            fc_back.v
        );
        assert!(
            (height_back - height).abs() < 2,
            "height roundtrip: {} -> {}",
            height,
            height_back
        );
    }

    #[test]
    fn test_terrain_height_displaces_outward() {
        let fc = FaceCoord::new(CubeFace::PosY, 0.5, 0.5);
        let pos_flat = face_uv_to_world_position(&fc, EARTH_RADIUS, 0, &ORIGIN);
        let pos_high = face_uv_to_world_position(&fc, EARTH_RADIUS, 1_000_000, &ORIGIN);

        let dist_flat = ((pos_flat.x as f64).powi(2)
            + (pos_flat.y as f64).powi(2)
            + (pos_flat.z as f64).powi(2))
        .sqrt();
        let dist_high = ((pos_high.x as f64).powi(2)
            + (pos_high.y as f64).powi(2)
            + (pos_high.z as f64).powi(2))
        .sqrt();

        assert!(
            dist_high > dist_flat + 999_000.0,
            "Height should increase distance from center: flat={dist_flat}, high={dist_high}"
        );
    }

    #[test]
    fn test_batch_grid_produces_correct_count() {
        let addr = ChunkAddress::new(CubeFace::PosX, 10, 50, 50);
        let resolution = 4;
        let heights: Vec<Vec<i64>> =
            vec![vec![0; (resolution + 1) as usize]; (resolution + 1) as usize];
        let positions =
            face_grid_to_world_positions(&addr, EARTH_RADIUS, &ORIGIN, resolution, &heights);
        let expected = ((resolution + 1) * (resolution + 1)) as usize;
        assert_eq!(positions.len(), expected);
    }

    #[test]
    fn test_batch_grid_points_on_sphere() {
        let addr = ChunkAddress::new(CubeFace::PosZ, 10, 50, 50);
        let resolution = 4;
        let heights: Vec<Vec<i64>> =
            vec![vec![0; (resolution + 1) as usize]; (resolution + 1) as usize];
        let positions =
            face_grid_to_world_positions(&addr, EARTH_RADIUS, &ORIGIN, resolution, &heights);
        for pos in &positions {
            let dist =
                ((pos.x as f64).powi(2) + (pos.y as f64).powi(2) + (pos.z as f64).powi(2)).sqrt();
            assert!(
                (dist - EARTH_RADIUS as f64).abs() < 10.0,
                "Grid point not on sphere: dist={dist}"
            );
        }
    }

    #[test]
    fn test_negative_terrain_height() {
        // Ocean: negative height should place point below surface
        let fc = FaceCoord::new(CubeFace::PosX, 0.5, 0.5);
        let pos_surface = face_uv_to_world_position(&fc, EARTH_RADIUS, 0, &ORIGIN);
        let pos_ocean = face_uv_to_world_position(&fc, EARTH_RADIUS, -1_000_000, &ORIGIN);

        let dist_surface = ((pos_surface.x as f64).powi(2)
            + (pos_surface.y as f64).powi(2)
            + (pos_surface.z as f64).powi(2))
        .sqrt();
        let dist_ocean = ((pos_ocean.x as f64).powi(2)
            + (pos_ocean.y as f64).powi(2)
            + (pos_ocean.z as f64).powi(2))
        .sqrt();

        assert!(
            dist_ocean < dist_surface - 999_000.0,
            "Negative height should decrease distance: surface={dist_surface}, ocean={dist_ocean}"
        );
    }
}
