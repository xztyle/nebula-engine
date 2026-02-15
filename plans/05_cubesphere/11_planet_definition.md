# Planet Definition

## Problem

The engine needs a canonical data structure to represent a planet — the central organizing entity of the cubesphere system. Every planet has a position in the universe, a radius, a procedural generation seed, and a name. Multiple planets must coexist in the same universe (solar systems, binary planets, moons). The engine needs a registry that can look up planets by name or by spatial proximity, ensure no two planets overlap, and provide the parameters that every other cubesphere system (chunk addressing, terrain generation, LOD, rendering) depends on. Without a formal `PlanetDef` type, planet parameters will be scattered across ad-hoc configuration files and hardcoded constants.

## Solution

Define planet data structures in the `nebula_cubesphere` crate (or a dedicated `nebula_planet` crate, depending on workspace organization).

### PlanetDef

```rust
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
    /// Earth-like: 6_371_000_000 (6,371 km in mm).
    /// Must be positive. Stored as i128 for consistency with WorldPosition,
    /// but always >= 1.
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
    pub fn new(name: impl Into<String>, center: WorldPosition, radius: i128, seed: u64) -> Self {
        assert!(radius > 0, "Planet radius must be positive, got {radius}");
        Self {
            name: name.into(),
            center,
            radius,
            seed,
        }
    }

    /// The surface area of the planet in mm^2 (approximate, treating it as a sphere).
    pub fn surface_area_mm2(&self) -> f64 {
        4.0 * std::f64::consts::PI * (self.radius as f64).powi(2)
    }

    /// The circumference of the planet in mm.
    pub fn circumference_mm(&self) -> f64 {
        2.0 * std::f64::consts::PI * self.radius as f64
    }

    /// Check whether a WorldPosition is inside the planet's base sphere
    /// (ignoring terrain).
    pub fn contains(&self, pos: &WorldPosition) -> bool {
        let dx = (pos.x - self.center.x) as f64;
        let dy = (pos.y - self.center.y) as f64;
        let dz = (pos.z - self.center.z) as f64;
        let dist_sq = dx * dx + dy * dy + dz * dz;
        dist_sq <= (self.radius as f64).powi(2)
    }
}
```

### Planet Registry

```rust
use std::collections::HashMap;

/// Registry of all planets in the current universe.
///
/// Provides lookup by name and validates that no two planets overlap.
pub struct PlanetRegistry {
    planets: Vec<PlanetDef>,
    name_index: HashMap<String, usize>,
}

impl PlanetRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            planets: Vec::new(),
            name_index: HashMap::new(),
        }
    }

    /// Register a new planet. Returns an error if:
    /// - A planet with the same name already exists
    /// - The new planet's sphere overlaps with an existing planet
    pub fn register(&mut self, planet: PlanetDef) -> Result<usize, PlanetRegistryError> {
        if self.name_index.contains_key(&planet.name) {
            return Err(PlanetRegistryError::DuplicateName(planet.name.clone()));
        }

        // Check for overlap with existing planets
        for existing in &self.planets {
            let dx = (planet.center.x - existing.center.x) as f64;
            let dy = (planet.center.y - existing.center.y) as f64;
            let dz = (planet.center.z - existing.center.z) as f64;
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            let min_dist = (planet.radius + existing.radius) as f64;
            if dist < min_dist {
                return Err(PlanetRegistryError::Overlap {
                    new: planet.name.clone(),
                    existing: existing.name.clone(),
                });
            }
        }

        let idx = self.planets.len();
        self.name_index.insert(planet.name.clone(), idx);
        self.planets.push(planet);
        Ok(idx)
    }

    /// Look up a planet by name.
    pub fn get_by_name(&self, name: &str) -> Option<&PlanetDef> {
        self.name_index.get(name).map(|&idx| &self.planets[idx])
    }

    /// Look up a planet by index.
    pub fn get_by_index(&self, idx: usize) -> Option<&PlanetDef> {
        self.planets.get(idx)
    }

    /// Number of registered planets.
    pub fn len(&self) -> usize {
        self.planets.len()
    }

    /// Returns true if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.planets.is_empty()
    }

    /// Iterate over all registered planets.
    pub fn iter(&self) -> impl Iterator<Item = &PlanetDef> {
        self.planets.iter()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlanetRegistryError {
    DuplicateName(String),
    Overlap { new: String, existing: String },
}

impl std::fmt::Display for PlanetRegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanetRegistryError::DuplicateName(name) =>
                write!(f, "Planet with name '{name}' already exists"),
            PlanetRegistryError::Overlap { new, existing } =>
                write!(f, "Planet '{new}' overlaps with existing planet '{existing}'"),
        }
    }
}

impl std::error::Error for PlanetRegistryError {}
```

### Common Planet Presets

```rust
impl PlanetDef {
    /// Earth-like planet preset.
    pub fn earth_like(name: impl Into<String>, center: WorldPosition, seed: u64) -> Self {
        Self::new(name, center, 6_371_000_000, seed) // 6,371 km in mm
    }

    /// Moon-like body preset.
    pub fn moon_like(name: impl Into<String>, center: WorldPosition, seed: u64) -> Self {
        Self::new(name, center, 1_737_400_000, seed) // 1,737.4 km in mm
    }

    /// Mars-like planet preset.
    pub fn mars_like(name: impl Into<String>, center: WorldPosition, seed: u64) -> Self {
        Self::new(name, center, 3_389_500_000, seed) // 3,389.5 km in mm
    }
}
```

### Design Constraints

- `PlanetDef` is a value type (data only, no behavior beyond queries). It does not own runtime state like chunk caches or mesh buffers.
- The `radius` is `i128` for consistency with `WorldPosition`, but in practice it fits in `i64` for any physically reasonable planet.
- The `seed` is `u64`, which provides 18.4 quintillion unique seeds — enough for any universe.
- The `PlanetRegistry` is not thread-safe by default; wrap in `Arc<RwLock<>>` for multi-threaded access (handled by the ECS integration, not this module).
- Planet names must be unique within a registry. No two planets may have the same name.

## Outcome

The `nebula_cubesphere` crate exports `PlanetDef`, `PlanetRegistry`, `PlanetRegistryError`, and planet preset constructors. Every system that interacts with a planet — terrain generation, chunk loading, rendering, physics, networking — reads from the `PlanetDef` to obtain the planet's parameters. Running `cargo test -p nebula_cubesphere` passes all planet definition and registry tests.

## Demo Integration

**Demo crate:** `nebula-demo`

The sphere is now an Earth-scale planet. The title bar shows `Planet: Terra, radius=6,371,000,000 mm`. The wireframe sphere is properly scaled.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_math` | workspace | `WorldPosition` type |

No external dependencies beyond the workspace. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_earth_radius_fits_in_i128() {
        let earth_radius: i128 = 6_371_000_000; // 6,371 km in mm
        assert!(earth_radius > 0);
        assert!(earth_radius < i128::MAX);

        let planet = PlanetDef::earth_like("Terra", WorldPosition::default(), 42);
        assert_eq!(planet.radius, earth_radius);
    }

    #[test]
    fn test_planet_center_plus_radius_no_overflow() {
        // Place a planet far from origin and verify no overflow
        let center = WorldPosition::new(
            i128::MAX / 4,
            i128::MAX / 4,
            i128::MAX / 4,
        );
        let radius: i128 = 6_371_000_000;
        let planet = PlanetDef::new("FarPlanet", center, radius, 1);

        // Surface point in +X direction
        let surface_x = planet.center.x.checked_add(planet.radius);
        assert!(surface_x.is_some(), "Center + radius overflowed");
    }

    #[test]
    fn test_registry_lookup_by_name() {
        let mut registry = PlanetRegistry::new();
        let terra = PlanetDef::earth_like("Terra", WorldPosition::default(), 42);
        registry.register(terra.clone()).unwrap();

        let found = registry.get_by_name("Terra");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "Terra");
        assert_eq!(found.unwrap().seed, 42);

        assert!(registry.get_by_name("Mars").is_none());
    }

    #[test]
    fn test_multiple_planets_distinct_centers() {
        let mut registry = PlanetRegistry::new();

        let terra = PlanetDef::earth_like(
            "Terra",
            WorldPosition::new(0, 0, 0),
            42,
        );
        let luna = PlanetDef::moon_like(
            "Luna",
            WorldPosition::new(384_400_000_000, 0, 0), // ~384,400 km from Terra
            43,
        );
        let mars = PlanetDef::mars_like(
            "Mars",
            WorldPosition::new(225_000_000_000_000, 0, 0), // ~225 million km
            44,
        );

        registry.register(terra).unwrap();
        registry.register(luna).unwrap();
        registry.register(mars).unwrap();

        assert_eq!(registry.len(), 3);

        let t = registry.get_by_name("Terra").unwrap();
        let l = registry.get_by_name("Luna").unwrap();
        let m = registry.get_by_name("Mars").unwrap();

        assert_ne!(t.center, l.center);
        assert_ne!(t.center, m.center);
        assert_ne!(l.center, m.center);
    }

    #[test]
    fn test_duplicate_name_rejected() {
        let mut registry = PlanetRegistry::new();
        let p1 = PlanetDef::earth_like("Terra", WorldPosition::default(), 1);
        let p2 = PlanetDef::earth_like("Terra", WorldPosition::new(999_999_999_999, 0, 0), 2);

        registry.register(p1).unwrap();
        let result = registry.register(p2);
        assert!(result.is_err());
        match result.unwrap_err() {
            PlanetRegistryError::DuplicateName(name) => assert_eq!(name, "Terra"),
            _ => panic!("Expected DuplicateName error"),
        }
    }

    #[test]
    fn test_overlapping_planets_rejected() {
        let mut registry = PlanetRegistry::new();
        let p1 = PlanetDef::earth_like("Terra", WorldPosition::default(), 1);
        // Place second planet so its sphere overlaps with Terra
        let p2 = PlanetDef::earth_like(
            "TooClose",
            WorldPosition::new(1_000_000_000, 0, 0), // 1000 km, well inside Earth radius
            2,
        );

        registry.register(p1).unwrap();
        let result = registry.register(p2);
        assert!(result.is_err());
        match result.unwrap_err() {
            PlanetRegistryError::Overlap { new, existing } => {
                assert_eq!(new, "TooClose");
                assert_eq!(existing, "Terra");
            }
            _ => panic!("Expected Overlap error"),
        }
    }

    #[test]
    fn test_planet_contains_point() {
        let planet = PlanetDef::earth_like("Terra", WorldPosition::default(), 42);

        // Point at center
        assert!(planet.contains(&WorldPosition::default()));

        // Point on surface
        let surface = WorldPosition::new(planet.radius, 0, 0);
        assert!(planet.contains(&surface));

        // Point outside
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
        // Earth circumference ≈ 40,075 km = 40,075,000,000 mm
        let expected = 2.0 * std::f64::consts::PI * 6_371_000_000.0;
        assert!((circumference - expected).abs() < 1.0);
    }
}
```
