//! Human-readable planetary coordinates: latitude, longitude, and altitude.
//!
//! Converts between [`WorldPosition`] (i128, mm) and [`PlanetaryCoord`] (lat/lon/alt)
//! using a [`PlanetBody`] reference frame. Integrates with the cubesphere terrain
//! system for accurate surface height queries.

use std::fmt;

use glam::DVec3;
use nebula_cubesphere::{FaceCoord, PlanetDef, direction_to_face_coord};
use nebula_math::WorldPosition;
use nebula_terrain::TerrainHeightSampler;

/// A position on a planet expressed as latitude, longitude, and altitude.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanetaryCoord {
    /// Latitude in degrees. Range: \[-90, 90\].
    /// Positive = north of the equator. Negative = south.
    pub latitude: f64,
    /// Longitude in degrees. Range: (-180, 180\].
    /// Positive = east. Negative = west.
    pub longitude: f64,
    /// Altitude above the terrain surface in meters.
    /// 0.0 means standing on the ground. Negative means underground.
    pub altitude: f64,
}

impl PlanetaryCoord {
    /// Create a new planetary coordinate.
    pub fn new(latitude: f64, longitude: f64, altitude: f64) -> Self {
        Self {
            latitude,
            longitude,
            altitude,
        }
    }

    /// Compute the great-circle (surface) distance to another coordinate.
    /// Uses the Haversine formula. Returns distance in meters.
    pub fn surface_distance_to(&self, other: &PlanetaryCoord, planet_radius_m: f64) -> f64 {
        let lat1 = self.latitude.to_radians();
        let lat2 = other.latitude.to_radians();
        let dlat = (other.latitude - self.latitude).to_radians();
        let dlon = (other.longitude - self.longitude).to_radians();

        let a = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
        let c = 2.0 * a.sqrt().asin();

        planet_radius_m * c
    }
}

impl fmt::Display for PlanetaryCoord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let lat_dir = if self.latitude >= 0.0 { "N" } else { "S" };
        let lon_dir = if self.longitude >= 0.0 { "E" } else { "W" };
        write!(
            f,
            "{:.1}\u{00B0}{}, {:.1}\u{00B0}{}, {:.0}m alt",
            self.latitude.abs(),
            lat_dir,
            self.longitude.abs(),
            lon_dir,
            self.altitude,
        )
    }
}

/// Planet center and metadata needed for coordinate conversion.
///
/// Combines a [`PlanetDef`] with a [`TerrainHeightSampler`] to convert
/// between world positions and human-readable lat/lon/alt.
pub struct PlanetBody<'a> {
    /// The planet definition (center, radius).
    pub def: &'a PlanetDef,
    /// Terrain sampler for querying surface height at a given direction.
    pub terrain: &'a TerrainHeightSampler,
}

impl<'a> PlanetBody<'a> {
    /// Create a new planet body reference.
    pub fn new(def: &'a PlanetDef, terrain: &'a TerrainHeightSampler) -> Self {
        Self { def, terrain }
    }

    /// Convert a world position to planetary coordinates (lat/lon/alt).
    pub fn world_to_planetary(&self, pos: &WorldPosition) -> PlanetaryCoord {
        let dx = (pos.x - self.def.center.x) as f64;
        let dy = (pos.y - self.def.center.y) as f64;
        let dz = (pos.z - self.def.center.z) as f64;
        let distance_mm = (dx * dx + dy * dy + dz * dz).sqrt();

        // Degenerate case: position is exactly at planet center.
        if distance_mm < 1e-10 {
            return PlanetaryCoord::new(0.0, 0.0, -(self.def.radius as f64) / 1000.0);
        }

        let dir = DVec3::new(dx, dy, dz) / distance_mm;

        // Spherical coordinates: latitude = arcsin(y), longitude = atan2(z, x).
        let latitude_deg = dir.y.asin().to_degrees();
        let longitude_deg = dir.z.atan2(dir.x).to_degrees();

        // Query terrain height at this direction (returns height in engine units = meters).
        let surface_height_m = self.terrain.sample_height(dir);

        // Ground distance from center in mm:
        // planet radius (mm) + terrain height converted to mm.
        let ground_distance_mm = self.def.radius as f64 + surface_height_m * 1000.0;
        let altitude_m = (distance_mm - ground_distance_mm) / 1000.0;

        PlanetaryCoord::new(latitude_deg, longitude_deg, altitude_m)
    }

    /// Convert planetary coordinates back to a world position.
    pub fn planetary_to_world(&self, coord: &PlanetaryCoord) -> WorldPosition {
        let lat_rad = coord.latitude.to_radians();
        let lon_rad = coord.longitude.to_radians();

        let dir = DVec3::new(
            lat_rad.cos() * lon_rad.cos(),
            lat_rad.sin(),
            lat_rad.cos() * lon_rad.sin(),
        );

        // Terrain surface height at this direction (meters).
        let surface_height_m = self.terrain.sample_height(dir);

        // Total distance from planet center in mm.
        let total_distance_mm =
            self.def.radius as f64 + surface_height_m * 1000.0 + coord.altitude * 1000.0;

        let world_offset = dir * total_distance_mm;

        WorldPosition::new(
            self.def.center.x + world_offset.x as i128,
            self.def.center.y + world_offset.y as i128,
            self.def.center.z + world_offset.z as i128,
        )
    }

    /// Convert a lat/lon to the corresponding cube face coordinate.
    pub fn latlon_to_face_coord(&self, latitude: f64, longitude: f64) -> FaceCoord {
        let lat_rad = latitude.to_radians();
        let lon_rad = longitude.to_radians();
        let dir = DVec3::new(
            lat_rad.cos() * lon_rad.cos(),
            lat_rad.sin(),
            lat_rad.cos() * lon_rad.sin(),
        );
        direction_to_face_coord(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_terrain::{HeightmapParams, TerrainHeightConfig};

    const EPSILON_DEG: f64 = 1e-6;
    const EPSILON_M: f64 = 0.001; // 1 mm

    fn flat_terrain() -> TerrainHeightSampler {
        TerrainHeightSampler::new(
            HeightmapParams {
                seed: 42,
                amplitude: 0.0,
                ..Default::default()
            },
            TerrainHeightConfig {
                min_height: 0.0,
                max_height: 0.0,
                sea_level: 0.0,
                planet_radius: 6_371_000.0,
            },
        )
    }

    fn test_planet_def() -> PlanetDef {
        PlanetDef::earth_like("TestPlanet", WorldPosition::default(), 42)
    }

    #[test]
    fn test_equator_is_latitude_zero() {
        let def = test_planet_def();
        let terrain = flat_terrain();
        let body = PlanetBody::new(&def, &terrain);
        let pos = WorldPosition::new(def.radius, 0, 0);
        let coord = body.world_to_planetary(&pos);
        assert!(
            coord.latitude.abs() < EPSILON_DEG,
            "Equator point should have latitude ~0, got {}",
            coord.latitude
        );
    }

    #[test]
    fn test_north_pole_is_latitude_90() {
        let def = test_planet_def();
        let terrain = flat_terrain();
        let body = PlanetBody::new(&def, &terrain);
        let pos = WorldPosition::new(0, def.radius, 0);
        let coord = body.world_to_planetary(&pos);
        assert!(
            (coord.latitude - 90.0).abs() < EPSILON_DEG,
            "North pole should have latitude ~90, got {}",
            coord.latitude
        );
    }

    #[test]
    fn test_south_pole_is_latitude_neg90() {
        let def = test_planet_def();
        let terrain = flat_terrain();
        let body = PlanetBody::new(&def, &terrain);
        let pos = WorldPosition::new(0, -def.radius, 0);
        let coord = body.world_to_planetary(&pos);
        assert!(
            (coord.latitude - (-90.0)).abs() < EPSILON_DEG,
            "South pole should have latitude ~-90, got {}",
            coord.latitude
        );
    }

    #[test]
    fn test_longitude_wraps_at_180() {
        let def = test_planet_def();
        let terrain = flat_terrain();
        let body = PlanetBody::new(&def, &terrain);
        let pos = WorldPosition::new(-def.radius, 0, 0);
        let coord = body.world_to_planetary(&pos);
        assert!(
            (coord.longitude.abs() - 180.0).abs() < EPSILON_DEG,
            "Negative X axis should have longitude ~+-180, got {}",
            coord.longitude
        );
    }

    #[test]
    fn test_altitude_zero_is_on_surface() {
        let def = test_planet_def();
        let terrain = flat_terrain();
        let body = PlanetBody::new(&def, &terrain);
        let pos = WorldPosition::new(def.radius, 0, 0);
        let coord = body.world_to_planetary(&pos);
        assert!(
            coord.altitude.abs() < EPSILON_M,
            "Point at planet radius should have altitude ~0, got {}",
            coord.altitude
        );
    }

    #[test]
    fn test_altitude_above_surface() {
        let def = test_planet_def();
        let terrain = flat_terrain();
        let body = PlanetBody::new(&def, &terrain);
        let altitude_mm = 10_000_000i128; // 10 km
        let pos = WorldPosition::new(def.radius + altitude_mm, 0, 0);
        let coord = body.world_to_planetary(&pos);
        assert!(
            (coord.altitude - 10_000.0).abs() < 1.0,
            "Expected altitude ~10000m, got {}",
            coord.altitude
        );
    }

    #[test]
    fn test_world_to_planetary_to_world_roundtrip() {
        let def = test_planet_def();
        let terrain = flat_terrain();
        let body = PlanetBody::new(&def, &terrain);

        let test_positions = [
            WorldPosition::new(def.radius, 0, 0),
            WorldPosition::new(0, def.radius, 0),
            WorldPosition::new(0, 0, def.radius),
            WorldPosition::new(
                (def.radius as f64 * 0.577) as i128,
                (def.radius as f64 * 0.577) as i128,
                (def.radius as f64 * 0.577) as i128,
            ),
            WorldPosition::new(def.radius + 50_000_000, 0, 0), // 50 km altitude
        ];

        for (i, original) in test_positions.iter().enumerate() {
            let planetary = body.world_to_planetary(original);
            let roundtrip = body.planetary_to_world(&planetary);

            let dx = (roundtrip.x - original.x).unsigned_abs();
            let dy = (roundtrip.y - original.y).unsigned_abs();
            let dz = (roundtrip.z - original.z).unsigned_abs();
            let error_mm =
                ((dx as f64) * (dx as f64) + (dy as f64) * (dy as f64) + (dz as f64) * (dz as f64))
                    .sqrt();

            assert!(
                error_mm < 10.0,
                "Roundtrip error for position {i}: {error_mm} mm \
                 (original={original:?}, roundtrip={roundtrip:?}, \
                 planetary={planetary})"
            );
        }
    }

    #[test]
    fn test_display_format() {
        let coord = PlanetaryCoord::new(45.3, -122.1, 150.0);
        let display = format!("{coord}");
        assert_eq!(display, "45.3\u{00B0}N, 122.1\u{00B0}W, 150m alt");

        let south_east = PlanetaryCoord::new(-23.4, 45.7, 0.0);
        let display = format!("{south_east}");
        assert_eq!(display, "23.4\u{00B0}S, 45.7\u{00B0}E, 0m alt");
    }

    #[test]
    fn test_great_circle_distance() {
        let new_york = PlanetaryCoord::new(40.7128, -74.0060, 0.0);
        let london = PlanetaryCoord::new(51.5074, -0.1278, 0.0);

        let distance_km = new_york.surface_distance_to(&london, 6_371_000.0) / 1000.0;

        assert!(
            (distance_km - 5570.0).abs() < 50.0,
            "NYC to London should be ~5570 km, got {distance_km} km"
        );
    }

    #[test]
    fn test_latlon_to_face_coord_equator() {
        let def = test_planet_def();
        let terrain = flat_terrain();
        let body = PlanetBody::new(&def, &terrain);

        let fc = body.latlon_to_face_coord(0.0, 0.0);
        // lat=0, lon=0 -> direction (1, 0, 0) -> PosX face, center
        assert_eq!(fc.face, nebula_cubesphere::CubeFace::PosX);
        assert!((fc.u - 0.5).abs() < 0.01);
        assert!((fc.v - 0.5).abs() < 0.01);
    }
}
