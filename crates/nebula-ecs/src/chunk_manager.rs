//! Chunk lifecycle management resource.

use bevy_ecs::prelude::*;
use std::collections::HashMap;

/// Manages the lifecycle of voxel chunks: which chunks are loaded,
/// which are pending generation, which should be unloaded.
///
/// The actual chunk data storage is in nebula-voxel. This resource
/// tracks chunk entities and their loading state.
#[derive(Resource, Debug, Default)]
pub struct ChunkManager {
    /// Map from chunk coordinate to the entity representing that chunk.
    pub loaded_chunks: HashMap<(i64, i64, i64), Entity>,
    /// Chunk coordinates queued for generation.
    pub pending_load: Vec<(i64, i64, i64)>,
    /// Chunk coordinates queued for unloading.
    pub pending_unload: Vec<(i64, i64, i64)>,
    /// The render distance in chunks.
    pub render_distance: u32,
}

impl ChunkManager {
    /// Creates a new [`ChunkManager`] with the given render distance.
    pub fn new(render_distance: u32) -> Self {
        Self {
            render_distance,
            ..Default::default()
        }
    }

    /// Returns true if a chunk at the given coordinate is loaded.
    pub fn is_loaded(&self, coord: (i64, i64, i64)) -> bool {
        self.loaded_chunks.contains_key(&coord)
    }

    /// Returns the entity for a loaded chunk, if any.
    pub fn chunk_entity(&self, coord: (i64, i64, i64)) -> Option<Entity> {
        self.loaded_chunks.get(&coord).copied()
    }

    /// Returns the number of currently loaded chunks.
    pub fn loaded_count(&self) -> usize {
        self.loaded_chunks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_manager_operations() {
        let mut cm = ChunkManager::new(8);
        assert_eq!(cm.render_distance, 8);
        assert_eq!(cm.loaded_count(), 0);
        assert!(!cm.is_loaded((0, 0, 0)));

        let mut world = World::new();
        let entity = world.spawn_empty().id();
        cm.loaded_chunks.insert((0, 0, 0), entity);

        assert!(cm.is_loaded((0, 0, 0)));
        assert_eq!(cm.chunk_entity((0, 0, 0)), Some(entity));
        assert_eq!(cm.loaded_count(), 1);
    }
}
