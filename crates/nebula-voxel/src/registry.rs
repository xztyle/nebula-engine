//! Voxel type registry: maps compact [`VoxelTypeId`] values to rich [`VoxelTypeDef`] metadata.
//!
//! The registry is built once during engine startup. Air is always ID 0 so that
//! zero-initialized chunk memory represents empty space.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Compact identifier stored inside every voxel cell (2 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VoxelTypeId(pub u16);

/// Transparency mode for a voxel type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transparency {
    /// Fully blocks light and visibility.
    Opaque,
    /// Partially transparent (e.g. water, stained glass).
    SemiTransparent,
    /// Completely transparent (e.g. air).
    FullyTransparent,
}

/// Full descriptor for a voxel type.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VoxelTypeDef {
    /// Human-readable name (e.g. "stone", "grass", "water").
    pub name: String,
    /// Whether entities collide with this voxel.
    pub solid: bool,
    /// Transparency mode.
    pub transparency: Transparency,
    /// Index into the material palette (albedo, roughness, etc.).
    pub material_index: u16,
    /// Light emission level (0 = none, 15 = max).
    pub light_emission: u8,
}

/// Errors that can occur during voxel type registration.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// A type with the same name has already been registered.
    #[error("duplicate voxel type name: {0}")]
    DuplicateName(String),
    /// All 65 535 user-defined slots have been consumed.
    #[error("voxel type registry is full (max 65536 types)")]
    RegistryFull,
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Maps [`VoxelTypeId`] → [`VoxelTypeDef`] with O(1) lookup by index and
/// O(1) reverse lookup by name.
pub struct VoxelTypeRegistry {
    /// Dense array where `index == VoxelTypeId.0`.
    types: Vec<VoxelTypeDef>,
    /// Reverse lookup: name → ID.
    name_to_id: HashMap<String, VoxelTypeId>,
}

impl VoxelTypeRegistry {
    /// Creates a new registry with Air pre-registered as ID 0.
    pub fn new() -> Self {
        let air = VoxelTypeDef {
            name: "air".to_string(),
            solid: false,
            transparency: Transparency::FullyTransparent,
            material_index: 0,
            light_emission: 0,
        };

        let mut name_to_id = HashMap::new();
        name_to_id.insert("air".to_string(), VoxelTypeId(0));

        Self {
            types: vec![air],
            name_to_id,
        }
    }

    /// Registers a new voxel type and returns its assigned ID.
    ///
    /// IDs are assigned sequentially starting from 1 (0 is Air).
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::DuplicateName`] if a type with the same name
    /// already exists, or [`RegistryError::RegistryFull`] if all 65 536 slots
    /// are consumed.
    pub fn register(&mut self, def: VoxelTypeDef) -> Result<VoxelTypeId, RegistryError> {
        if self.name_to_id.contains_key(&def.name) {
            return Err(RegistryError::DuplicateName(def.name));
        }
        if self.types.len() > u16::MAX as usize {
            return Err(RegistryError::RegistryFull);
        }

        let id = VoxelTypeId(self.types.len() as u16);
        self.name_to_id.insert(def.name.clone(), id);
        self.types.push(def);
        Ok(id)
    }

    /// Returns the definition for a given ID.
    ///
    /// # Panics
    ///
    /// Panics if `id` is out of range — this indicates a programming error
    /// since IDs are only produced by the registry itself.
    pub fn get(&self, id: VoxelTypeId) -> &VoxelTypeDef {
        &self.types[id.0 as usize]
    }

    /// Returns the ID for a named voxel type, or `None` if not found.
    pub fn lookup_by_name(&self, name: &str) -> Option<VoxelTypeId> {
        self.name_to_id.get(name).copied()
    }

    /// Returns the total number of registered types (including Air).
    pub fn len(&self) -> usize {
        self.types.len()
    }

    /// Returns `true` if only Air is registered.
    pub fn is_empty(&self) -> bool {
        self.types.len() <= 1
    }

    /// Returns `true` if the given voxel type is air (ID 0).
    pub fn is_air(&self, id: VoxelTypeId) -> bool {
        id.0 == 0
    }

    /// Returns `true` if the given voxel type is transparent (fully or semi).
    ///
    /// Air is considered transparent. Returns `true` for unknown IDs as a
    /// conservative fallback (treat missing types like air).
    pub fn is_transparent(&self, id: VoxelTypeId) -> bool {
        match self.types.get(id.0 as usize) {
            Some(def) => def.transparency != Transparency::Opaque,
            None => true,
        }
    }
}

impl Default for VoxelTypeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn stone_def() -> VoxelTypeDef {
        VoxelTypeDef {
            name: "stone".to_string(),
            solid: true,
            transparency: Transparency::Opaque,
            material_index: 1,
            light_emission: 0,
        }
    }

    fn dirt_def() -> VoxelTypeDef {
        VoxelTypeDef {
            name: "dirt".to_string(),
            solid: true,
            transparency: Transparency::Opaque,
            material_index: 2,
            light_emission: 0,
        }
    }

    fn grass_def() -> VoxelTypeDef {
        VoxelTypeDef {
            name: "grass".to_string(),
            solid: true,
            transparency: Transparency::Opaque,
            material_index: 3,
            light_emission: 0,
        }
    }

    #[test]
    fn test_air_is_id_zero() {
        let registry = VoxelTypeRegistry::new();
        let air = registry.get(VoxelTypeId(0));
        assert_eq!(air.name, "air");
        assert!(!air.solid);
        assert_eq!(air.transparency, Transparency::FullyTransparent);
    }

    #[test]
    fn test_register_returns_sequential_ids() {
        let mut registry = VoxelTypeRegistry::new();
        let id1 = registry.register(stone_def()).unwrap();
        let id2 = registry.register(dirt_def()).unwrap();
        let id3 = registry.register(grass_def()).unwrap();
        assert_eq!(id1, VoxelTypeId(1));
        assert_eq!(id2, VoxelTypeId(2));
        assert_eq!(id3, VoxelTypeId(3));
    }

    #[test]
    fn test_lookup_by_name() {
        let mut registry = VoxelTypeRegistry::new();
        let obsidian = VoxelTypeDef {
            name: "obsidian".to_string(),
            solid: true,
            transparency: Transparency::Opaque,
            material_index: 10,
            light_emission: 0,
        };
        let id = registry.register(obsidian).unwrap();
        assert_eq!(registry.lookup_by_name("obsidian"), Some(id));
        assert_eq!(registry.lookup_by_name("nonexistent"), None);
    }

    #[test]
    fn test_get_returns_correct_def() {
        let mut registry = VoxelTypeRegistry::new();
        let def = VoxelTypeDef {
            name: "glowstone".to_string(),
            solid: true,
            transparency: Transparency::Opaque,
            material_index: 42,
            light_emission: 12,
        };
        let id = registry.register(def).unwrap();
        let retrieved = registry.get(id);
        assert!(retrieved.solid);
        assert_eq!(retrieved.material_index, 42);
        assert_eq!(retrieved.light_emission, 12);
    }

    #[test]
    fn test_duplicate_name_rejected() {
        let mut registry = VoxelTypeRegistry::new();
        registry.register(stone_def()).unwrap();
        let result = registry.register(stone_def());
        assert!(matches!(result, Err(RegistryError::DuplicateName(_))));
    }

    #[test]
    fn test_len() {
        let mut registry = VoxelTypeRegistry::new();
        assert_eq!(registry.len(), 1); // Air
        registry.register(stone_def()).unwrap();
        assert_eq!(registry.len(), 2);
    }
}
