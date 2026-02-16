//! Planet definition — the canonical data structure for a planet's immutable parameters.

use nebula_math::WorldPosition;

/// Definition of a planet in the universe.
///
/// This is the immutable specification of a planet. It does not contain
/// runtime state (loaded chunks, mesh caches, etc.) — those belong to
/// the planet's ECS entity or a `PlanetState` companion struct.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanetDef {
    /// Unique human-readable name (e.g., "Terra", "Luna", "Kepler-442b").
    pub name: String,

    /// Center position in universe space (i128, mm).
    pub center: WorldPosition,

    /// Radius of the planet's base sphere in mm (before terrain displacement).
    ///
    /// Earth-like: `6_371_000_000` (6,371 km in mm).
    /// Must be positive.
    pub radius: i128,

    /// Seed for all procedural generation on this planet (terrain, biomes,
    /// caves, ore distribution, vegetation).
    ///
    /// Combined with chunk addresses to produce deterministic, reproducible
    /// terrain for any chunk on the planet.
    pub seed: u64,
}

impl PlanetDef {
    /// Construct a new planet definition.
    ///
    /// # Panics
    ///
    /// Panics if `radius` is not positive.
    pub fn new(name: impl Into<String>, center: WorldPosition, radius: i128, seed: u64) -> Self {
        assert!(radius > 0, "Planet radius must be positive, got {radius}");
        Self {
            name: name.into(),
            center,
            radius,
            seed,
        }
    }

    /// Earth-like planet preset (radius 6,371 km).
    pub fn earth_like(name: impl Into<String>, center: WorldPosition, seed: u64) -> Self {
        Self::new(name, center, 6_371_000_000, seed)
    }

    /// Moon-like body preset (radius 1,737.4 km).
    pub fn moon_like(name: impl Into<String>, center: WorldPosition, seed: u64) -> Self {
        Self::new(name, center, 1_737_400_000, seed)
    }

    /// Mars-like planet preset (radius 3,389.5 km).
    pub fn mars_like(name: impl Into<String>, center: WorldPosition, seed: u64) -> Self {
        Self::new(name, center, 3_389_500_000, seed)
    }

    /// The surface area of the planet in mm² (approximate, treating it as a sphere).
    pub fn surface_area_mm2(&self) -> f64 {
        4.0 * std::f64::consts::PI * (self.radius as f64).powi(2)
    }

    /// The circumference of the planet in mm.
    pub fn circumference_mm(&self) -> f64 {
        2.0 * std::f64::consts::PI * self.radius as f64
    }

    /// Check whether a `WorldPosition` is inside the planet's base sphere
    /// (ignoring terrain).
    pub fn contains(&self, pos: &WorldPosition) -> bool {
        let dx = (pos.x - self.center.x) as f64;
        let dy = (pos.y - self.center.y) as f64;
        let dz = (pos.z - self.center.z) as f64;
        let dist_sq = dx * dx + dy * dy + dz * dz;
        dist_sq <= (self.radius as f64).powi(2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_earth_radius_fits_in_i128() {
        let earth_radius: i128 = 6_371_000_000;
        assert!(earth_radius > 0);
        assert!(earth_radius < i128::MAX);

        let planet = PlanetDef::earth_like("Terra", WorldPosition::default(), 42);
        assert_eq!(planet.radius, earth_radius);
    }

    #[test]
    fn test_planet_center_plus_radius_no_overflow() {
        let center = WorldPosition::new(i128::MAX / 4, i128::MAX / 4, i128::MAX / 4);
        let radius: i128 = 6_371_000_000;
        let planet = PlanetDef::new("FarPlanet", center, radius, 1);

        let surface_x = planet.center.x.checked_add(planet.radius);
        assert!(surface_x.is_some(), "Center + radius overflowed");
    }

    #[test]
    fn test_planet_contains_point() {
        let planet = PlanetDef::earth_like("Terra", WorldPosition::default(), 42);

        assert!(planet.contains(&WorldPosition::default()));

        let surface = WorldPosition::new(planet.radius, 0, 0);
        assert!(planet.contains(&surface));

        let outside = WorldPosition::new(planet.radius + 1_000_000, 0, 0);
        assert!(!planet.contains(&outside));
    }

    #[test]
    #[should_panic(expected = "radius must be positive")]
    fn test_zero_radius_panics() {
        PlanetDef::new("BadPlanet", WorldPosition::default(), 0, 1);
    }

    #[test]
    fn test_planet_circumference() {
        let planet = PlanetDef::earth_like("Terra", WorldPosition::default(), 42);
        let circumference = planet.circumference_mm();
        let expected = 2.0 * std::f64::consts::PI * 6_371_000_000.0;
        assert!((circumference - expected).abs() < 1.0);
    }

    #[test]
    fn test_planet_surface_area() {
        let planet = PlanetDef::earth_like("Terra", WorldPosition::default(), 42);
        let area = planet.surface_area_mm2();
        let expected = 4.0 * std::f64::consts::PI * (6_371_000_000.0_f64).powi(2);
        assert!((area - expected).abs() / expected < 1e-10);
    }

    #[test]
    fn test_presets() {
        let earth = PlanetDef::earth_like("E", WorldPosition::default(), 1);
        assert_eq!(earth.radius, 6_371_000_000);

        let moon = PlanetDef::moon_like("M", WorldPosition::default(), 2);
        assert_eq!(moon.radius, 1_737_400_000);

        let mars = PlanetDef::mars_like("R", WorldPosition::default(), 3);
        assert_eq!(mars.radius, 3_389_500_000);
    }
}
