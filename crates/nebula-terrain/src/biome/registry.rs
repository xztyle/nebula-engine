//! Biome registry: maps [`BiomeId`] to [`BiomeDef`] with name-based lookup.

use hashbrown::HashMap;

use super::BiomeDef;

/// Unique identifier for a biome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BiomeId(pub u16);

/// Errors that can occur when registering biomes.
#[derive(Debug, thiserror::Error)]
pub enum BiomeRegistryError {
    /// A biome with this name is already registered.
    #[error("duplicate biome name: {0}")]
    DuplicateName(String),
}

/// Stores all registered biome definitions with O(1) lookup by ID.
pub struct BiomeRegistry {
    biomes: Vec<BiomeDef>,
    name_to_id: HashMap<String, BiomeId>,
}

impl BiomeRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            biomes: Vec::new(),
            name_to_id: HashMap::new(),
        }
    }

    /// Registers a new biome definition, returning its assigned [`BiomeId`].
    ///
    /// # Errors
    ///
    /// Returns [`BiomeRegistryError::DuplicateName`] if a biome with the same name exists.
    pub fn register(&mut self, def: BiomeDef) -> Result<BiomeId, BiomeRegistryError> {
        if self.name_to_id.contains_key(&def.name) {
            return Err(BiomeRegistryError::DuplicateName(def.name.clone()));
        }
        let id = BiomeId(self.biomes.len() as u16);
        self.name_to_id.insert(def.name.clone(), id);
        self.biomes.push(def);
        Ok(id)
    }

    /// Returns the definition for the given biome ID.
    ///
    /// # Panics
    ///
    /// Panics if `id` is out of range.
    pub fn get(&self, id: BiomeId) -> &BiomeDef {
        &self.biomes[id.0 as usize]
    }

    /// Looks up a biome ID by name.
    pub fn lookup_by_name(&self, name: &str) -> Option<BiomeId> {
        self.name_to_id.get(name).copied()
    }

    /// Returns the number of registered biomes.
    pub fn len(&self) -> usize {
        self.biomes.len()
    }

    /// Returns `true` if no biomes are registered.
    pub fn is_empty(&self) -> bool {
        self.biomes.is_empty()
    }
}

impl Default for BiomeRegistry {
    fn default() -> Self {
        Self::new()
    }
}
