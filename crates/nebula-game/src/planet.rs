//! Planet configuration for Elite Dangerous Lite.
//!
//! Defines the Earth-scale planet parameters used by the game.
//! Earth radius: 6,371 km = 6,371,000 m.
//! Starting altitude: 400 km above the surface (ISS orbit).

use nebula_config::PlanetConfig;

/// Earth radius in meters.
pub const EARTH_RADIUS_M: f64 = 6_371_000.0;

/// ISS orbital altitude in meters (~400 km).
const ISS_ALTITUDE_M: f64 = 400_000.0;

/// Camera speed at orbital altitude in meters per second (~8 km/s, orbital velocity).
const ORBITAL_CAMERA_SPEED: f64 = 8_000.0;

/// Create the planet configuration for an Earth-scale world.
///
/// Planet center is at the origin. Camera starts 400 km above the north pole.
pub fn earth_config() -> PlanetConfig {
    PlanetConfig {
        radius_m: EARTH_RADIUS_M,
        start_altitude_m: ISS_ALTITUDE_M,
        free_fly_camera: true,
        camera_speed_m_s: ORBITAL_CAMERA_SPEED,
    }
}
