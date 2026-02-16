//! Registry of all known voxel/block types.

use bevy_ecs::prelude::*;

/// Registry of all known voxel/block types. Maps block IDs to their
/// properties (name, is_solid, is_transparent, texture indices, etc.).
///
/// This resource is populated at startup and may be extended at runtime
/// by mods or procedural generation. It is read-only during normal
/// simulation — only Update may add new types.
#[derive(Resource, Debug, Default)]
pub struct VoxelRegistry {
    entries: Vec<VoxelTypeEntry>,
}

/// Describes a single voxel/block type.
#[derive(Debug, Clone)]
pub struct VoxelTypeEntry {
    /// Numeric ID assigned by the registry.
    pub id: u16,
    /// Human-readable name.
    pub name: String,
    /// Whether this block is solid (blocks movement/light).
    pub is_solid: bool,
    /// Whether this block is transparent (allows light through).
    pub is_transparent: bool,
}

impl VoxelRegistry {
    /// Registers a new voxel type and returns its assigned ID.
    /// The `id` field of the entry is overwritten with the actual assigned ID.
    pub fn register(&mut self, entry: VoxelTypeEntry) -> u16 {
        let id = self.entries.len() as u16;
        self.entries.push(VoxelTypeEntry { id, ..entry });
        id
    }

    /// Returns the entry for the given block ID, if it exists.
    pub fn get(&self, id: u16) -> Option<&VoxelTypeEntry> {
        self.entries.get(id as usize)
    }

    /// Returns the number of registered voxel types.
    pub fn count(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_voxel_registry_register_and_get() {
        let mut registry = VoxelRegistry::default();
        assert_eq!(registry.count(), 0);

        let id = registry.register(VoxelTypeEntry {
            id: 0,
            name: "stone".to_string(),
            is_solid: true,
            is_transparent: false,
        });

        assert_eq!(id, 0);
        assert_eq!(registry.count(), 1);

        let entry = registry.get(id).unwrap();
        assert_eq!(entry.name, "stone");
        assert!(entry.is_solid);
        assert!(!entry.is_transparent);
    }

    #[test]
    fn test_voxel_registry_multiple_types() {
        let mut registry = VoxelRegistry::default();
        let air = registry.register(VoxelTypeEntry {
            id: 0,
            name: "air".to_string(),
            is_solid: false,
            is_transparent: true,
        });
        let stone = registry.register(VoxelTypeEntry {
            id: 0,
            name: "stone".to_string(),
            is_solid: true,
            is_transparent: false,
        });
        let glass = registry.register(VoxelTypeEntry {
            id: 0,
            name: "glass".to_string(),
            is_solid: true,
            is_transparent: true,
        });

        assert_eq!(air, 0);
        assert_eq!(stone, 1);
        assert_eq!(glass, 2);
        assert_eq!(registry.count(), 3);
        assert!(registry.get(stone).unwrap().is_solid);
        assert!(registry.get(glass).unwrap().is_transparent);
    }
}
