//! Planet registry — lookup and validation for all planets in a universe.

use std::collections::HashMap;

use crate::PlanetDef;

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

    /// Register a new planet. Returns the planet's index on success.
    ///
    /// # Errors
    ///
    /// Returns an error if a planet with the same name already exists or if the
    /// new planet's sphere overlaps with an existing planet.
    pub fn register(&mut self, planet: PlanetDef) -> Result<usize, PlanetRegistryError> {
        if self.name_index.contains_key(&planet.name) {
            return Err(PlanetRegistryError::DuplicateName(planet.name.clone()));
        }

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

impl Default for PlanetRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors that can occur when registering a planet.
#[derive(Clone, Debug, PartialEq)]
pub enum PlanetRegistryError {
    /// A planet with this name already exists.
    DuplicateName(String),
    /// The new planet's sphere overlaps with an existing planet.
    Overlap {
        /// Name of the planet being registered.
        new: String,
        /// Name of the existing planet it overlaps with.
        existing: String,
    },
}

impl std::fmt::Display for PlanetRegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateName(name) => write!(f, "Planet with name '{name}' already exists"),
            Self::Overlap { new, existing } => {
                write!(
                    f,
                    "Planet '{new}' overlaps with existing planet '{existing}'"
                )
            }
        }
    }
}

impl std::error::Error for PlanetRegistryError {}

#[cfg(test)]
mod tests {
    use nebula_math::WorldPosition;

    use super::*;

    #[test]
    fn test_registry_lookup_by_name() {
        let mut registry = PlanetRegistry::new();
        let terra = PlanetDef::earth_like("Terra", WorldPosition::default(), 42);
        registry.register(terra).unwrap();

        let found = registry.get_by_name("Terra");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "Terra");
        assert_eq!(found.unwrap().seed, 42);

        assert!(registry.get_by_name("Mars").is_none());
    }

    #[test]
    fn test_multiple_planets_distinct_centers() {
        let mut registry = PlanetRegistry::new();

        let terra = PlanetDef::earth_like("Terra", WorldPosition::new(0, 0, 0), 42);
        let luna = PlanetDef::moon_like("Luna", WorldPosition::new(384_400_000_000, 0, 0), 43);
        let mars = PlanetDef::mars_like("Mars", WorldPosition::new(225_000_000_000_000, 0, 0), 44);

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
        let p2 = PlanetDef::earth_like("TooClose", WorldPosition::new(1_000_000_000, 0, 0), 2);

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
    fn test_get_by_index() {
        let mut registry = PlanetRegistry::new();
        let terra = PlanetDef::earth_like("Terra", WorldPosition::default(), 42);
        let idx = registry.register(terra).unwrap();

        let found = registry.get_by_index(idx).unwrap();
        assert_eq!(found.name, "Terra");
        assert!(registry.get_by_index(999).is_none());
    }

    #[test]
    fn test_empty_registry() {
        let registry = PlanetRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert_eq!(registry.iter().count(), 0);
    }

    #[test]
    fn test_default_registry() {
        let registry = PlanetRegistry::default();
        assert!(registry.is_empty());
    }
}
