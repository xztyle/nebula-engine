# World Save System

## Problem

The engine can save and load individual scenes (story 02), but a complete game world encompasses far more than a single scene's entities. A world save must capture: player state (inventory, position, stats), all modified terrain chunks (voxel edits, placed blocks), all persistent entity state (NPCs, items, vehicles), and world metadata (world time, seed, game rules). These different data types have different storage characteristics -- entity state is best as a single structured file, while chunk data is bulk binary that should be stored per-chunk for efficient partial loading.

Saving every chunk on a planet with millions of generated chunks is infeasible. Only chunks that the player has actually modified (built, mined, placed objects) need to be saved. Unmodified chunks can be regenerated deterministically from the world seed. This means the save system must integrate with the chunk dirty-tracking system (from the voxel module) to identify which chunks need persistence.

A typical save for a mid-game world might include: 1 world metadata file (~1 KB), 1 entity state file (~100 KB for 2,000 entities), and 200-500 dirty chunk files (~1-4 KB each, ~500 KB total). The entire save should fit in under 2 MB uncompressed.

## Solution

Implement a world save system in `nebula_save` that writes a structured directory of files.

### Save Directory Layout

```
saves/
  <save_name>/
    world.ron          -- World metadata (seed, time, version, game rules)
    entities.bin       -- All persistent entities (postcard binary)
    chunks/
      <face>_<x>_<y>_<lod>.chunk  -- Binary chunk files, only for dirty chunks
```

### World Metadata

```rust
use serde::{Serialize, Deserialize};

/// World-level metadata persisted in `world.ron`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldMeta {
    /// Save format version for migration (see story 06).
    pub save_version: u32,
    /// The world seed used for procedural generation. Unmodified chunks
    /// are regenerated from this seed rather than stored.
    pub seed: u64,
    /// In-game time in seconds since world creation.
    pub world_time: f64,
    /// Human-readable save name for the UI.
    pub display_name: String,
    /// Wall-clock timestamp of when the save was created.
    pub created_at: String,
    /// Wall-clock timestamp of the most recent save.
    pub last_saved_at: String,
    /// Total play time in seconds.
    pub play_time: f64,
}

pub const CURRENT_SAVE_VERSION: u32 = 1;
```

### Chunk File Naming

Chunk files are named by their address to allow direct lookup:

```rust
use std::path::PathBuf;

/// Build the file path for a chunk within the save directory.
/// Format: `<face>_<x>_<y>_<lod>.chunk`
pub fn chunk_file_path(
    save_dir: &std::path::Path,
    address: &ChunkAddress,
) -> PathBuf {
    save_dir
        .join("chunks")
        .join(format!(
            "{}_{}_{}_{}.chunk",
            address.face as u8,
            address.x,
            address.y,
            address.lod,
        ))
}
```

### Save Function

```rust
use std::fs;
use std::io::Write;
use tracing::{info, warn};

/// Save the entire game world to the specified directory.
///
/// - Writes `world.ron` with metadata.
/// - Writes `entities.bin` with all persistent entities.
/// - Writes one `.chunk` file per dirty chunk in `chunks/`.
/// - Compresses chunk files with LZ4 if they exceed 256 bytes.
pub fn save_world(
    save_dir: &std::path::Path,
    world: &World,
    registry: &ComponentRegistry,
    chunk_manager: &ChunkManager,
    meta: &WorldMeta,
) -> Result<SaveStats, SaveError> {
    let mut stats = SaveStats::default();

    // Ensure directory structure exists
    fs::create_dir_all(save_dir.join("chunks"))?;

    // 1. Write world metadata as RON
    let meta_ron = ron::ser::to_string_pretty(meta, ron::ser::PrettyConfig::default())
        .map_err(|e| SaveError::Serialize(e.to_string()))?;
    fs::write(save_dir.join("world.ron"), &meta_ron)?;
    stats.meta_bytes = meta_ron.len();

    // 2. Save persistent entities to binary
    let scene = save_scene(world, registry);
    let entity_bytes = scene.to_binary()?;
    fs::write(save_dir.join("entities.bin"), &entity_bytes)?;
    stats.entity_bytes = entity_bytes.len();
    stats.entity_count = scene.entities.len();

    // 3. Save dirty chunks
    for (address, chunk) in chunk_manager.dirty_chunks() {
        let raw = chunk.serialize();
        let compressed = lz4_flex::compress_prepend_size(&raw);
        let path = chunk_file_path(save_dir, &address);
        fs::write(&path, &compressed)?;
        stats.chunk_count += 1;
        stats.chunk_bytes += compressed.len();
    }

    info!(
        "World saved: {} entities ({} bytes), {} chunks ({} bytes)",
        stats.entity_count, stats.entity_bytes,
        stats.chunk_count, stats.chunk_bytes,
    );

    Ok(stats)
}

/// Statistics from a save operation.
#[derive(Debug, Default)]
pub struct SaveStats {
    pub meta_bytes: usize,
    pub entity_count: usize,
    pub entity_bytes: usize,
    pub chunk_count: usize,
    pub chunk_bytes: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serialize(String),
    #[error("scene error: {0}")]
    Scene(#[from] SceneSerError),
}
```

### Load Function

```rust
/// Load a world save from disk and populate the ECS world.
///
/// - Reads `world.ron` for metadata.
/// - Reads `entities.bin` and spawns all persistent entities.
/// - Chunk files are NOT loaded eagerly -- they are loaded on demand
///   when the chunk manager requests a chunk that has a save file.
///   This function registers the available chunk files so the chunk
///   manager knows which addresses have saved data.
pub fn load_world(
    save_dir: &std::path::Path,
    world: &mut World,
    registry: &ComponentRegistry,
) -> Result<LoadedWorld, SaveError> {
    // 1. Read and parse world metadata
    let meta_str = fs::read_to_string(save_dir.join("world.ron"))?;
    let meta: WorldMeta = ron::from_str(&meta_str)
        .map_err(|e| SaveError::Serialize(e.to_string()))?;

    // 2. Load entities
    let entity_bytes = fs::read(save_dir.join("entities.bin"))?;
    let scene = SceneData::from_binary(&entity_bytes)?;
    let spawned = load_scene(world, &scene, registry, LoadMode::Replace)?;

    // 3. Discover saved chunk files (do not load them yet)
    let mut saved_chunks: Vec<ChunkAddress> = Vec::new();
    let chunks_dir = save_dir.join("chunks");
    if chunks_dir.exists() {
        for entry in fs::read_dir(&chunks_dir)? {
            let entry = entry?;
            let file_name = entry.file_name();
            if let Some(address) = parse_chunk_filename(&file_name.to_string_lossy()) {
                saved_chunks.push(address);
            }
        }
    }

    Ok(LoadedWorld {
        meta,
        spawned_entities: spawned,
        saved_chunk_addresses: saved_chunks,
    })
}

pub struct LoadedWorld {
    pub meta: WorldMeta,
    pub spawned_entities: Vec<Entity>,
    pub saved_chunk_addresses: Vec<ChunkAddress>,
}

/// Load a single chunk from its save file. Called on demand by the
/// chunk manager when a chunk with saved data enters the loading radius.
pub fn load_chunk(
    save_dir: &std::path::Path,
    address: &ChunkAddress,
) -> Result<ChunkData, SaveError> {
    let path = chunk_file_path(save_dir, address);
    let compressed = fs::read(&path)?;
    let raw = lz4_flex::decompress_size_prepended(&compressed)
        .map_err(|e| SaveError::Serialize(e.to_string()))?;
    let chunk = ChunkData::deserialize(&raw)
        .map_err(|e| SaveError::Serialize(e.to_string()))?;
    Ok(chunk)
}

/// Parse a chunk filename like "2_15_-3_0.chunk" into a ChunkAddress.
fn parse_chunk_filename(name: &str) -> Option<ChunkAddress> {
    let stem = name.strip_suffix(".chunk")?;
    let parts: Vec<&str> = stem.split('_').collect();
    if parts.len() != 4 { return None; }
    Some(ChunkAddress {
        face: parts[0].parse().ok()?,
        x: parts[1].parse().ok()?,
        y: parts[2].parse().ok()?,
        lod: parts[3].parse().ok()?,
    })
}
```

### Design Decisions

- **Directory-per-save over single archive**: A directory structure allows partial reads (load metadata without loading entities), parallel chunk I/O, and incremental saves (overwrite only changed chunks). A single archive (zip, tar) would require rewriting the entire file for any change.
- **Lazy chunk loading**: Chunk files are discovered at load time but only actually read when the chunk enters the player's loading radius. This keeps the initial load time constant regardless of how many chunks have been modified.
- **LZ4 compression**: LZ4 is chosen for its decompression speed (>4 GB/s) which is critical for chunk loading during gameplay. The compression ratio is modest (~2:1 for typical chunk data) but the speed tradeoff is worth it for a game engine.
- **Dirty-only chunk saving**: Unmodified chunks are regenerated from the world seed. This keeps save sizes proportional to the player's impact on the world, not the world's total size. A million generated chunks produce zero save data until the player modifies one.

## Outcome

A `save_world()` function that writes a structured save directory containing world metadata (RON), entity state (postcard binary), and dirty chunk files (LZ4-compressed binary). A `load_world()` function that restores the world state with lazy chunk loading. Only modified chunks are persisted; unmodified chunks are regenerated from the seed.

## Demo Integration

**Demo crate:** `nebula-demo`

The full world is saved: all chunks, entities, and modifications. The save is a directory of files organized by region.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | ECS World access for entity serialization |
| `serde` | `1.0` | Serialize/Deserialize for WorldMeta and scene data |
| `ron` | `0.12` | Human-readable metadata serialization |
| `postcard` | `1.1` | Binary entity serialization |
| `lz4_flex` | `0.11` | Fast LZ4 compression for chunk files |
| `thiserror` | `2.0` | Error type derivation |
| `tracing` | `0.1` | Logging save statistics |

Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_world_save_writes_files() {
        let tmp = TempDir::new().unwrap();
        let save_dir = tmp.path().join("test_save");

        let mut world = World::new();
        let registry = ComponentRegistry::default();
        let chunk_manager = ChunkManager::new_empty();
        let meta = WorldMeta {
            save_version: CURRENT_SAVE_VERSION,
            seed: 12345,
            world_time: 3600.0,
            display_name: "Test World".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            last_saved_at: "2026-01-01T01:00:00Z".into(),
            play_time: 3600.0,
        };

        save_world(&save_dir, &world, &registry, &chunk_manager, &meta).unwrap();

        // Verify files exist
        assert!(save_dir.join("world.ron").exists());
        assert!(save_dir.join("entities.bin").exists());
        assert!(save_dir.join("chunks").is_dir());
    }

    #[test]
    fn test_load_restores_state() {
        let tmp = TempDir::new().unwrap();
        let save_dir = tmp.path().join("roundtrip_save");

        // Create world with a persistent entity
        let mut world = World::new();
        let registry = test_registry();
        world.spawn((
            Persistent,
            Health { current: 42, max: 100 },
        ));

        let chunk_manager = ChunkManager::new_empty();
        let meta = WorldMeta {
            save_version: CURRENT_SAVE_VERSION,
            seed: 99,
            world_time: 0.0,
            display_name: "Test".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            last_saved_at: "2026-01-01T00:00:00Z".into(),
            play_time: 0.0,
        };

        save_world(&save_dir, &world, &registry, &chunk_manager, &meta).unwrap();

        // Load into a fresh world
        let mut new_world = World::new();
        let loaded = load_world(&save_dir, &mut new_world, &registry).unwrap();

        assert_eq!(loaded.meta.seed, 99);
        assert_eq!(loaded.spawned_entities.len(), 1);
        let health = new_world.get::<Health>(loaded.spawned_entities[0]).unwrap();
        assert_eq!(health.current, 42);
    }

    #[test]
    fn test_only_modified_chunks_are_saved() {
        let tmp = TempDir::new().unwrap();
        let save_dir = tmp.path().join("dirty_chunks");

        let world = World::new();
        let registry = ComponentRegistry::default();

        // Create a chunk manager with 3 chunks, only 1 dirty
        let mut chunk_manager = ChunkManager::new_empty();
        chunk_manager.add_chunk(ChunkAddress::test(0, 0, 0, 0), false); // clean
        chunk_manager.add_chunk(ChunkAddress::test(0, 1, 0, 0), true);  // dirty
        chunk_manager.add_chunk(ChunkAddress::test(0, 2, 0, 0), false); // clean

        let meta = WorldMeta::test_default();
        let stats = save_world(&save_dir, &world, &registry, &chunk_manager, &meta).unwrap();

        assert_eq!(stats.chunk_count, 1, "Only the dirty chunk should be saved");
    }

    #[test]
    fn test_regenerated_chunks_match_originals() {
        // Generate a chunk from seed, verify it is deterministic
        let seed = 42u64;
        let address = ChunkAddress::test(0, 5, 5, 0);

        let chunk1 = generate_chunk_from_seed(seed, &address);
        let chunk2 = generate_chunk_from_seed(seed, &address);

        // Same seed + same address = identical chunk
        assert_eq!(chunk1.serialize(), chunk2.serialize());
    }

    #[test]
    fn test_save_directory_structure_is_correct() {
        let tmp = TempDir::new().unwrap();
        let save_dir = tmp.path().join("structure_test");

        let world = World::new();
        let registry = ComponentRegistry::default();
        let mut chunk_manager = ChunkManager::new_empty();
        chunk_manager.add_chunk(ChunkAddress::test(2, 10, -5, 0), true);

        let meta = WorldMeta::test_default();
        save_world(&save_dir, &world, &registry, &chunk_manager, &meta).unwrap();

        assert!(save_dir.join("world.ron").is_file());
        assert!(save_dir.join("entities.bin").is_file());
        assert!(save_dir.join("chunks").is_dir());
        assert!(save_dir.join("chunks/2_10_-5_0.chunk").is_file());
    }
}
```
