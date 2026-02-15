# Planetary Coordinate System

## Problem

Players, game systems, and debugging tools need a human-readable way to describe positions on a planet's surface. "You are at WorldPosition (4,502,238,917,392, 4,502,238,917,392, 0)" is useless. What players need is latitude, longitude, and altitude -- the same system used to describe positions on Earth. The engine uses a cubesphere, not a perfect sphere, so the mapping from cube face coordinates to latitude/longitude is not trivial. Additionally, "altitude" should mean distance above the terrain surface (not above the mathematical sphere), which requires querying the terrain heightmap at the given lat/lon. The coordinate system must support conversion in both directions: world position to planetary coordinates (for display) and planetary coordinates to world position (for teleportation, navigation, map markers). The roundtrip must be lossless to the precision of the underlying types.

## Solution

### Planetary Coordinate Type

```rust
use std::fmt;

/// A position on a planet expressed as latitude, longitude, and altitude.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanetaryCoord {
    /// Latitude in degrees. Range: [-90, 90].
    /// Positive = north of the equator. Negative = south.
    pub latitude: f64,
    /// Longitude in degrees. Range: (-180, 180].
    /// Positive = east. Negative = west.
    pub longitude: f64,
    /// Altitude above the terrain surface in meters.
    /// 0.0 means standing on the ground. Negative means underground.
    pub altitude: f64,
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
```

### World Position to Planetary Coordinates

Converting a world-space position (i128) to lat/lon/alt requires:

1. Compute the direction from the planet center to the world position.
2. Convert the direction to spherical coordinates (lat/lon).
3. Compute the distance from the planet center (radius).
4. Query the terrain heightmap at that lat/lon to get the surface elevation.
5. Altitude = distance from center - (planet_radius + surface_elevation).

```rust
use nebula_coords::WorldPosition;
use glam::DVec3;

/// Planet center and metadata needed for coordinate conversion.
pub struct PlanetBody {
    /// Center of the planet in world space.
    pub center: WorldPosition,
    /// Base radius of the planet in millimeters.
    pub radius_mm: i128,
    /// Terrain sampler for querying surface height at a given direction.
    pub terrain: HeightmapSampler,
}

impl PlanetBody {
    /// Convert a world position to planetary coordinates (lat/lon/alt).
    pub fn world_to_planetary(&self, pos: &WorldPosition) -> PlanetaryCoord {
        // Direction from planet center to position, in f64.
        let dx = (pos.x - self.center.x) as f64;
        let dy = (pos.y - self.center.y) as f64;
        let dz = (pos.z - self.center.z) as f64;
        let distance_mm = (dx * dx + dy * dy + dz * dz).sqrt();

        // Normalize to get the unit direction.
        let dir = DVec3::new(dx, dy, dz) / distance_mm;

        // Spherical coordinates: latitude = arcsin(y), longitude = atan2(z, x).
        let latitude_rad = dir.y.asin();
        let longitude_rad = dir.z.atan2(dir.x);

        let latitude_deg = latitude_rad.to_degrees();
        let longitude_deg = longitude_rad.to_degrees();

        // Query terrain height at this direction.
        let surface_height_mm = self.terrain.sample_3d(dir) * 1000.0; // Convert m to mm

        // Altitude = distance from center - (radius + terrain height).
        let ground_distance_mm = self.radius_mm as f64 + surface_height_mm;
        let altitude_m = (distance_mm - ground_distance_mm) / 1000.0; // mm to meters

        PlanetaryCoord {
            latitude: latitude_deg,
            longitude: longitude_deg,
            altitude: altitude_m,
        }
    }

    /// Convert planetary coordinates back to a world position.
    pub fn planetary_to_world(&self, coord: &PlanetaryCoord) -> WorldPosition {
        let lat_rad = coord.latitude.to_radians();
        let lon_rad = coord.longitude.to_radians();

        // Unit direction from planet center.
        let dir = DVec3::new(
            lat_rad.cos() * lon_rad.cos(),
            lat_rad.sin(),
            lat_rad.cos() * lon_rad.sin(),
        );

        // Terrain surface height at this direction.
        let surface_height_mm = self.terrain.sample_3d(dir) * 1000.0;

        // Total distance from planet center in mm.
        let total_distance_mm =
            self.radius_mm as f64 + surface_height_mm + coord.altitude * 1000.0;

        let world_offset = dir * total_distance_mm;

        WorldPosition {
            x: self.center.x + world_offset.x as i128,
            y: self.center.y + world_offset.y as i128,
            z: self.center.z + world_offset.z as i128,
        }
    }
}
```

### Cubesphere Integration

The latitude/longitude system maps naturally to the cubesphere. Latitude corresponds to the elevation angle from the equatorial plane (Y axis), and longitude corresponds to the azimuthal angle around the Y axis. The cubesphere's `sphere_to_cube_inverse()` function (Epic 05, story 03) can convert a lat/lon direction to a `FaceCoord` for terrain queries, ensuring that the terrain heightmap lookup is consistent with the cubesphere geometry:

```rust
use nebula_cubesphere::{sphere_to_cube_inverse, FaceCoord};

impl PlanetBody {
    /// Convert a lat/lon to the corresponding cube face coordinate.
    pub fn latlon_to_face_coord(&self, latitude: f64, longitude: f64) -> FaceCoord {
        let lat_rad = latitude.to_radians();
        let lon_rad = longitude.to_radians();
        let dir = DVec3::new(
            lat_rad.cos() * lon_rad.cos(),
            lat_rad.sin(),
            lat_rad.cos() * lon_rad.sin(),
        );
        sphere_to_cube_inverse(&dir)
    }
}
```

### Longitude Wrapping

Longitude must wrap correctly at the +-180 degree boundary. `atan2` naturally returns values in `(-PI, PI]`, which maps to `(-180, 180]` degrees. The conversion functions handle this without special casing.

### Human-Readable Display

The `Display` implementation formats coordinates as:

- `45.3N, 122.1W, 150m alt` -- positive latitude = North, positive longitude = East
- `23.4S, 45.7E, 0m alt` -- at sea level in the southern hemisphere
- `89.9N, 0.0E, -50m alt` -- underground near the north pole

### Distance Between Planetary Coordinates

For convenience, provide a great-circle distance function:

```rust
impl PlanetaryCoord {
    /// Compute the great-circle (surface) distance to another coordinate.
    /// Uses the Haversine formula. Returns distance in meters.
    pub fn surface_distance_to(&self, other: &PlanetaryCoord, planet_radius_m: f64) -> f64 {
        let lat1 = self.latitude.to_radians();
        let lat2 = other.latitude.to_radians();
        let dlat = (other.latitude - self.latitude).to_radians();
        let dlon = (other.longitude - self.longitude).to_radians();

        let a = (dlat / 2.0).sin().powi(2)
            + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
        let c = 2.0 * a.sqrt().asin();

        planet_radius_m * c
    }
}
```

## Outcome

The `nebula-planet` crate exports `PlanetaryCoord`, `PlanetBody`, and the conversion functions `world_to_planetary()` and `planetary_to_world()`. Positions on a planet surface can be expressed as human-readable latitude/longitude/altitude strings. The roundtrip conversion (world -> planetary -> world) preserves the position to within 1 millimeter. The coordinate system integrates with the cubesphere terrain system for accurate surface height queries.

## Demo Integration

**Demo crate:** `nebula-demo`

The debug HUD shows the camera's position as latitude, longitude, and altitude (e.g., `45.32N, 122.67W, alt 1234m`). The coordinate updates in real time as the camera moves.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | `0.29` | `DVec3` for direction vectors, trigonometric operations |

Internal dependencies: `nebula-coords` (WorldPosition), `nebula-cubesphere` (sphere_to_cube_inverse, FaceCoord), `nebula-terrain` (HeightmapSampler). Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_coords::WorldPosition;

    const EPSILON_DEG: f64 = 1e-6;
    const EPSILON_M: f64 = 0.001; // 1 mm

    fn test_planet() -> PlanetBody {
        PlanetBody {
            center: WorldPosition::ZERO,
            radius_mm: 6_371_000_000, // 6371 km
            terrain: HeightmapSampler::flat(0.0), // Flat terrain for predictable tests
        }
    }

    #[test]
    fn test_equator_is_latitude_zero() {
        let planet = test_planet();
        // A point on the equator: positive X axis, no Y component.
        let pos = WorldPosition {
            x: planet.radius_mm,
            y: 0,
            z: 0,
        };
        let coord = planet.world_to_planetary(&pos);
        assert!(
            coord.latitude.abs() < EPSILON_DEG,
            "Equator point should have latitude ~0, got {}",
            coord.latitude
        );
    }

    #[test]
    fn test_north_pole_is_latitude_90() {
        let planet = test_planet();
        // North pole: positive Y axis.
        let pos = WorldPosition {
            x: 0,
            y: planet.radius_mm,
            z: 0,
        };
        let coord = planet.world_to_planetary(&pos);
        assert!(
            (coord.latitude - 90.0).abs() < EPSILON_DEG,
            "North pole should have latitude ~90, got {}",
            coord.latitude
        );
    }

    #[test]
    fn test_south_pole_is_latitude_neg90() {
        let planet = test_planet();
        // South pole: negative Y axis.
        let pos = WorldPosition {
            x: 0,
            y: -planet.radius_mm,
            z: 0,
        };
        let coord = planet.world_to_planetary(&pos);
        assert!(
            (coord.latitude - (-90.0)).abs() < EPSILON_DEG,
            "South pole should have latitude ~-90, got {}",
            coord.latitude
        );
    }

    #[test]
    fn test_longitude_wraps_at_180() {
        let planet = test_planet();
        // Negative X axis should be longitude +-180.
        let pos = WorldPosition {
            x: -planet.radius_mm,
            y: 0,
            z: 0,
        };
        let coord = planet.world_to_planetary(&pos);
        assert!(
            (coord.longitude.abs() - 180.0).abs() < EPSILON_DEG,
            "Negative X axis should have longitude ~+-180, got {}",
            coord.longitude
        );
    }

    #[test]
    fn test_altitude_zero_is_on_surface() {
        let planet = test_planet();
        // Point exactly at planet radius on the equator.
        let pos = WorldPosition {
            x: planet.radius_mm,
            y: 0,
            z: 0,
        };
        let coord = planet.world_to_planetary(&pos);
        assert!(
            coord.altitude.abs() < EPSILON_M,
            "Point at planet radius should have altitude ~0, got {}",
            coord.altitude
        );
    }

    #[test]
    fn test_altitude_above_surface() {
        let planet = test_planet();
        let altitude_mm = 10_000_000; // 10 km
        let pos = WorldPosition {
            x: planet.radius_mm + altitude_mm,
            y: 0,
            z: 0,
        };
        let coord = planet.world_to_planetary(&pos);
        assert!(
            (coord.altitude - 10_000.0).abs() < 1.0, // Within 1 meter
            "Expected altitude ~10000m, got {}",
            coord.altitude
        );
    }

    #[test]
    fn test_world_to_planetary_to_world_roundtrip() {
        let planet = test_planet();

        // Test a variety of positions around the planet.
        let test_positions = [
            WorldPosition { x: planet.radius_mm, y: 0, z: 0 },
            WorldPosition { x: 0, y: planet.radius_mm, z: 0 },
            WorldPosition { x: 0, y: 0, z: planet.radius_mm },
            WorldPosition {
                x: (planet.radius_mm as f64 * 0.577) as i128,
                y: (planet.radius_mm as f64 * 0.577) as i128,
                z: (planet.radius_mm as f64 * 0.577) as i128,
            },
            WorldPosition {
                x: planet.radius_mm + 50_000_000, // 50 km altitude
                y: 0,
                z: 0,
            },
        ];

        for (i, original) in test_positions.iter().enumerate() {
            let planetary = planet.world_to_planetary(original);
            let roundtrip = planet.planetary_to_world(&planetary);

            let dx = (roundtrip.x - original.x).abs();
            let dy = (roundtrip.y - original.y).abs();
            let dz = (roundtrip.z - original.z).abs();
            let error_mm = ((dx * dx + dy * dy + dz * dz) as f64).sqrt();

            assert!(
                error_mm < 10.0, // Within 10 mm (allows for f64 rounding)
                "Roundtrip error for position {i}: {error_mm} mm \
                 (original={original:?}, roundtrip={roundtrip:?}, \
                 planetary={planetary})"
            );
        }
    }

    #[test]
    fn test_display_format() {
        let coord = PlanetaryCoord {
            latitude: 45.3,
            longitude: -122.1,
            altitude: 150.0,
        };
        let display = format!("{coord}");
        assert_eq!(display, "45.3\u{00B0}N, 122.1\u{00B0}W, 150m alt");

        let south_east = PlanetaryCoord {
            latitude: -23.4,
            longitude: 45.7,
            altitude: 0.0,
        };
        let display = format!("{south_east}");
        assert_eq!(display, "23.4\u{00B0}S, 45.7\u{00B0}E, 0m alt");
    }

    #[test]
    fn test_great_circle_distance() {
        let new_york = PlanetaryCoord {
            latitude: 40.7128,
            longitude: -74.0060,
            altitude: 0.0,
        };
        let london = PlanetaryCoord {
            latitude: 51.5074,
            longitude: -0.1278,
            altitude: 0.0,
        };

        let distance_km = new_york.surface_distance_to(&london, 6_371_000.0) / 1000.0;

        // Real-world distance is approximately 5570 km.
        assert!(
            (distance_km - 5570.0).abs() < 50.0,
            "NYC to London should be ~5570 km, got {distance_km} km"
        );
    }
}
```
