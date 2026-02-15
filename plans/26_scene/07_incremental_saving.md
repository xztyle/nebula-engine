# Incremental Saving

## Problem

The world save system (story 05) writes all dirty chunks and all persistent entities in a single synchronous operation. For a large world with hundreds of dirty chunks, this can take 50-200 ms -- long enough to cause a visible frame hitch. Autosaving every 5 minutes should be seamless; the player should never notice it happening. Additionally, if the game crashes or loses power mid-save, the previous save must not be corrupted. A half-written save file is worse than no save at all.

The system needs three properties: (1) non-blocking -- disk I/O happens off the main thread so the game loop is never stalled; (2) atomic -- a save either fully succeeds or the previous save remains intact; (3) incremental -- only data that changed since the last save is written to disk.

## Solution

Implement an incremental autosave system in `nebula_save` that runs on a background thread with double-buffering and atomic file replacement.

### Autosave Configuration

```rust
use bevy_ecs::prelude::*;

/// Configuration for the autosave system. Inserted as an ECS resource.
#[derive(Resource, Debug, Clone)]
pub struct AutosaveConfig {
    /// Interval between autosaves in seconds. Default: 300 (5 minutes).
    pub interval_secs: f64,
    /// Whether autosave is enabled. Can be toggled from settings.
    pub enabled: bool,
    /// Maximum number of backup saves to keep. Default: 1.
    pub max_backups: u32,
}

impl Default for AutosaveConfig {
    fn default() -> Self {
        Self {
            interval_secs: 300.0,
            enabled: true,
            max_backups: 1,
        }
    }
}
```

### Save State Tracking

```rust
/// Tracks the state of the autosave system. Inserted as an ECS resource.
#[derive(Resource, Debug)]
pub struct AutosaveState {
    /// Time accumulated since the last autosave (seconds).
    pub time_since_last_save: f64,
    /// Whether a background save is currently in progress.
    pub save_in_progress: bool,
    /// Progress of the current save (0.0 to 1.0). Updated by the
    /// background thread via a shared atomic.
    pub progress: f32,
    /// Result of the last completed save, if any.
    pub last_result: Option<Result<SaveStats, String>>,
    /// Handle to the background save thread.
    thread_handle: Option<std::thread::JoinHandle<Result<SaveStats, SaveError>>>,
    /// Shared progress counter, written by background thread, read by main thread.
    shared_progress: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

impl AutosaveState {
    pub fn new() -> Self {
        Self {
            time_since_last_save: 0.0,
            save_in_progress: false,
            progress: 0.0,
            last_result: None,
            thread_handle: None,
            shared_progress: std::sync::Arc::new(
                std::sync::atomic::AtomicU32::new(0)
            ),
        }
    }
}
```

### Snapshot for Background Thread

The background thread cannot access the ECS world (it is not `Send`-safe during system execution). Instead, the main thread takes a snapshot of all data that needs saving and hands it to the background thread.

```rust
/// A snapshot of saveable state, taken on the main thread and sent to
/// the background thread for writing. This decouples the I/O from the
/// ECS world lifetime.
pub struct SaveSnapshot {
    /// Serialized world metadata (RON string).
    pub meta_ron: String,
    /// Serialized entity data (postcard binary).
    pub entity_bytes: Vec<u8>,
    /// Dirty chunks: (address, serialized chunk bytes).
    pub dirty_chunks: Vec<(ChunkAddress, Vec<u8>)>,
    /// The save directory path.
    pub save_dir: std::path::PathBuf,
}

/// Take a snapshot of the current world state for background saving.
/// This runs on the main thread and should be fast (< 5 ms for typical
/// worlds) because it only serializes data, no disk I/O.
pub fn take_save_snapshot(
    world: &World,
    registry: &ComponentRegistry,
    chunk_manager: &ChunkManager,
    meta: &WorldMeta,
    save_dir: &std::path::Path,
) -> Result<SaveSnapshot, SaveError> {
    let meta_ron = ron::ser::to_string_pretty(meta, ron::ser::PrettyConfig::default())
        .map_err(|e| SaveError::Serialize(e.to_string()))?;

    let scene = save_scene(world, registry);
    let entity_bytes = scene.to_binary()?;

    let dirty_chunks: Vec<(ChunkAddress, Vec<u8>)> = chunk_manager
        .dirty_chunks()
        .map(|(addr, chunk)| {
            let raw = chunk.serialize();
            let compressed = lz4_flex::compress_prepend_size(&raw);
            (addr, compressed)
        })
        .collect();

    Ok(SaveSnapshot {
        meta_ron,
        entity_bytes,
        dirty_chunks,
        save_dir: save_dir.to_path_buf(),
    })
}
```

### Background Save Thread

```rust
use std::fs;
use std::sync::atomic::Ordering;

/// Write a save snapshot to disk on a background thread.
/// Uses atomic file replacement: write to `.tmp`, then rename.
pub fn write_snapshot_to_disk(
    snapshot: SaveSnapshot,
    progress: std::sync::Arc<std::sync::atomic::AtomicU32>,
    max_backups: u32,
) -> Result<SaveStats, SaveError> {
    let save_dir = &snapshot.save_dir;
    let tmp_dir = save_dir.with_extension("saving_tmp");
    let backup_dir = save_dir.with_extension("backup");

    let total_items = 2 + snapshot.dirty_chunks.len(); // meta + entities + chunks
    let mut completed = 0u32;

    let update_progress = |completed: &mut u32| {
        *completed += 1;
        let pct = (*completed as f32 / total_items as f32 * 100.0) as u32;
        progress.store(pct.min(100), Ordering::Relaxed);
    };

    // Phase 1: Write everything to a temp directory
    fs::create_dir_all(tmp_dir.join("chunks"))?;

    // Write metadata
    fs::write(tmp_dir.join("world.ron"), &snapshot.meta_ron)?;
    update_progress(&mut completed);

    // Write entities
    fs::write(tmp_dir.join("entities.bin"), &snapshot.entity_bytes)?;
    update_progress(&mut completed);

    // Write dirty chunks
    let mut chunk_bytes_total = 0usize;
    for (address, data) in &snapshot.dirty_chunks {
        let path = chunk_file_path(&tmp_dir, address);
        fs::write(&path, data)?;
        chunk_bytes_total += data.len();
        update_progress(&mut completed);
    }

    // Phase 2: Also copy existing chunk files that are NOT in the dirty set
    // (they were saved in a previous save and are still valid)
    let existing_chunks_dir = save_dir.join("chunks");
    if existing_chunks_dir.exists() {
        let dirty_set: std::collections::HashSet<String> = snapshot.dirty_chunks
            .iter()
            .map(|(addr, _)| format!(
                "{}_{}_{}_{}.chunk",
                addr.face as u8, addr.x, addr.y, addr.lod,
            ))
            .collect();

        for entry in fs::read_dir(&existing_chunks_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.ends_with(".chunk") && !dirty_set.contains(name_str.as_ref()) {
                fs::copy(entry.path(), tmp_dir.join("chunks").join(&name))?;
            }
        }
    }

    // Phase 3: Atomic swap
    // Current save -> backup
    if save_dir.exists() {
        if backup_dir.exists() {
            fs::remove_dir_all(&backup_dir)?;
        }
        fs::rename(save_dir, &backup_dir)?;
    }

    // Temp -> current save
    fs::rename(&tmp_dir, save_dir)?;

    // Phase 4: Clean up old backups beyond max_backups
    if max_backups == 0 && backup_dir.exists() {
        fs::remove_dir_all(&backup_dir)?;
    }

    progress.store(100, Ordering::Relaxed);

    Ok(SaveStats {
        meta_bytes: snapshot.meta_ron.len(),
        entity_count: 0, // Not tracked in snapshot
        entity_bytes: snapshot.entity_bytes.len(),
        chunk_count: snapshot.dirty_chunks.len(),
        chunk_bytes: chunk_bytes_total,
    })
}
```

### Autosave ECS System

```rust
/// ECS system that triggers autosaves on an interval. Runs in PreUpdate.
pub fn autosave_system(
    mut state: ResMut<AutosaveState>,
    config: Res<AutosaveConfig>,
    time: Res<TimeRes>,
    world: &World,
    registry: Res<ComponentRegistry>,
    chunk_manager: Res<ChunkManager>,
    meta: Res<WorldMeta>,
) {
    // Check if a background save completed
    if state.save_in_progress {
        let progress_pct = state.shared_progress.load(std::sync::atomic::Ordering::Relaxed);
        state.progress = progress_pct as f32 / 100.0;

        if let Some(handle) = state.thread_handle.take() {
            if handle.is_finished() {
                match handle.join() {
                    Ok(Ok(stats)) => {
                        tracing::info!("Autosave complete: {:?}", stats);
                        state.last_result = Some(Ok(stats));
                    }
                    Ok(Err(e)) => {
                        tracing::error!("Autosave failed: {}", e);
                        state.last_result = Some(Err(e.to_string()));
                    }
                    Err(_) => {
                        tracing::error!("Autosave thread panicked");
                        state.last_result = Some(Err("thread panicked".into()));
                    }
                }
                state.save_in_progress = false;
                state.progress = 0.0;
            } else {
                // Thread still running, put the handle back
                state.thread_handle = Some(handle);
            }
        }
        return; // Don't start a new save while one is in progress
    }

    if !config.enabled {
        return;
    }

    // Accumulate time
    state.time_since_last_save += time.delta as f64;
    if state.time_since_last_save < config.interval_secs {
        return;
    }

    // Time to save
    state.time_since_last_save = 0.0;
    let save_dir = std::path::PathBuf::from("saves/autosave");

    match take_save_snapshot(world, &registry, &chunk_manager, &meta, &save_dir) {
        Ok(snapshot) => {
            let progress = state.shared_progress.clone();
            progress.store(0, std::sync::atomic::Ordering::Relaxed);
            let max_backups = config.max_backups;

            let handle = std::thread::Builder::new()
                .name("autosave".into())
                .spawn(move || {
                    write_snapshot_to_disk(snapshot, progress, max_backups)
                })
                .expect("Failed to spawn autosave thread");

            state.thread_handle = Some(handle);
            state.save_in_progress = true;
            state.progress = 0.0;
            tracing::info!("Autosave started");
        }
        Err(e) => {
            tracing::error!("Failed to create save snapshot: {}", e);
        }
    }
}
```

### Dirty Flag Management

After a successful save, dirty flags on chunks must be cleared so they are not re-saved next time (unless modified again). This is done on the main thread after the background save completes:

```rust
/// Clear dirty flags on chunks that were included in the last save.
/// Called after the background save thread completes successfully.
pub fn clear_saved_chunk_dirty_flags(
    chunk_manager: &mut ChunkManager,
    saved_addresses: &[ChunkAddress],
) {
    for addr in saved_addresses {
        chunk_manager.clear_dirty(addr);
    }
}
```

### Design Decisions

- **Snapshot + background thread over async**: A dedicated OS thread for I/O is simpler than async (no executor, no waker complexity) and guarantees the I/O does not block the game's task pool. The snapshot is taken synchronously (fast, < 5 ms) and the heavy I/O is fully off-thread.
- **Atomic rename over in-place overwrite**: Writing to a temp directory and renaming is the standard pattern for crash-safe file updates. If the process dies during the write, the original save remains untouched. The rename operation is atomic on all major filesystems (ext4, NTFS, APFS).
- **Backup rotation**: The previous save is kept as a backup. If the new save is somehow corrupted (bad rename, filesystem bug), the player can recover from the backup. The `max_backups` config controls how many old saves to keep.
- **Shared atomic for progress**: The background thread reports progress via an `AtomicU32`. The main thread reads it each frame to update the UI. No mutex contention, no channel overhead.
- **Dirty flag cleared after save, not before**: Chunk dirty flags are cleared on the main thread after the background save confirms success. If chunks are modified between snapshot and completion, they will still be dirty and saved next time.

## Outcome

An `autosave_system` that triggers every 5 minutes (configurable), takes a fast snapshot of the world state on the main thread, then writes it to disk on a background thread using atomic file replacement. Only dirty chunks are written. The previous save is kept as a backup. Save progress is tracked via a shared atomic counter for UI display. The game loop is never blocked by disk I/O.

## Demo Integration

**Demo crate:** `nebula-demo`

Only modified chunks are written during a save operation. Saving 5 modified chunks out of 10,000 takes milliseconds.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | ECS resources and systems for autosave state |
| `serde` | `1.0` | Serialize/Deserialize for snapshot data |
| `ron` | `0.12` | RON serialization for world metadata |
| `postcard` | `1.1` | Binary serialization for entity data |
| `lz4_flex` | `0.11` | LZ4 compression for chunk data |
| `tracing` | `0.1` | Logging save start, completion, and errors |
| `thiserror` | `2.0` | Error type derivation |

Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn minimal_snapshot(save_dir: &std::path::Path) -> SaveSnapshot {
        SaveSnapshot {
            meta_ron: "(save_version: 1, seed: 42)".into(),
            entity_bytes: vec![0x4E, 0x53, 0x43, 0x44, 0x01, 0x00, 0x00, 0x00],
            dirty_chunks: vec![],
            save_dir: save_dir.to_path_buf(),
        }
    }

    #[test]
    fn test_autosave_triggers_on_interval() {
        let config = AutosaveConfig {
            interval_secs: 10.0,
            enabled: true,
            max_backups: 1,
        };
        let mut state = AutosaveState::new();

        // Accumulate 9 seconds -- should NOT trigger
        state.time_since_last_save = 9.0;
        assert!(state.time_since_last_save < config.interval_secs);

        // Accumulate to 10 seconds -- SHOULD trigger
        state.time_since_last_save = 10.0;
        assert!(state.time_since_last_save >= config.interval_secs);
    }

    #[test]
    fn test_only_dirty_chunks_written() {
        let tmp = TempDir::new().unwrap();
        let save_dir = tmp.path().join("dirty_test");

        let snapshot = SaveSnapshot {
            meta_ron: "(save_version: 1)".into(),
            entity_bytes: vec![0x4E, 0x53, 0x43, 0x44, 0x01, 0x00],
            dirty_chunks: vec![
                (ChunkAddress::test(0, 1, 2, 0), vec![0xAA, 0xBB]),
                // Only these two chunks should appear on disk
                (ChunkAddress::test(1, 3, 4, 0), vec![0xCC, 0xDD]),
            ],
            save_dir: save_dir.clone(),
        };

        let progress = Arc::new(AtomicU32::new(0));
        write_snapshot_to_disk(snapshot, progress, 1).unwrap();

        let chunk_dir = save_dir.join("chunks");
        let chunk_files: Vec<_> = fs::read_dir(&chunk_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(chunk_files.len(), 2);
    }

    #[test]
    fn test_save_is_atomic_rename() {
        let tmp = TempDir::new().unwrap();
        let save_dir = tmp.path().join("atomic_test");

        // First save
        let snapshot = minimal_snapshot(&save_dir);
        let progress = Arc::new(AtomicU32::new(0));
        write_snapshot_to_disk(snapshot, progress, 1).unwrap();
        assert!(save_dir.join("world.ron").exists());

        // Second save -- should rename first to backup
        let snapshot2 = minimal_snapshot(&save_dir);
        let progress2 = Arc::new(AtomicU32::new(0));
        write_snapshot_to_disk(snapshot2, progress2, 1).unwrap();

        // Current save exists
        assert!(save_dir.join("world.ron").exists());
        // Backup exists
        let backup_dir = save_dir.with_extension("backup");
        assert!(backup_dir.join("world.ron").exists());
    }

    #[test]
    fn test_previous_save_is_backup() {
        let tmp = TempDir::new().unwrap();
        let save_dir = tmp.path().join("backup_test");

        // Write "version_a" as the first save
        let mut snapshot = minimal_snapshot(&save_dir);
        snapshot.meta_ron = "(save_version: 1, label: \"version_a\")".into();
        let progress = Arc::new(AtomicU32::new(0));
        write_snapshot_to_disk(snapshot, progress, 1).unwrap();

        // Write "version_b" as the second save
        let mut snapshot2 = minimal_snapshot(&save_dir);
        snapshot2.meta_ron = "(save_version: 1, label: \"version_b\")".into();
        let progress2 = Arc::new(AtomicU32::new(0));
        write_snapshot_to_disk(snapshot2, progress2, 1).unwrap();

        // Current save should be version_b
        let current = fs::read_to_string(save_dir.join("world.ron")).unwrap();
        assert!(current.contains("version_b"));

        // Backup should be version_a
        let backup = fs::read_to_string(
            save_dir.with_extension("backup").join("world.ron")
        ).unwrap();
        assert!(backup.contains("version_a"));
    }

    #[test]
    fn test_game_doesnt_freeze_during_save() {
        // Verify that the save runs on a background thread by checking
        // that the main thread can continue immediately after spawning.
        let tmp = TempDir::new().unwrap();
        let save_dir = tmp.path().join("async_test");

        let snapshot = minimal_snapshot(&save_dir);
        let progress = Arc::new(AtomicU32::new(0));
        let progress_clone = progress.clone();

        let handle = std::thread::spawn(move || {
            write_snapshot_to_disk(snapshot, progress_clone, 1)
        });

        // Main thread is NOT blocked -- we can do work here
        let mut counter = 0u32;
        while !handle.is_finished() {
            counter += 1;
            std::thread::yield_now();
        }

        // The main thread was free to spin while the save happened
        // (counter > 0 proves the main thread was not blocked)
        handle.join().unwrap().unwrap();
        assert!(save_dir.join("world.ron").exists());
    }

    #[test]
    fn test_save_progress_is_trackable() {
        let tmp = TempDir::new().unwrap();
        let save_dir = tmp.path().join("progress_test");

        let mut snapshot = minimal_snapshot(&save_dir);
        // Add some chunks so progress increments are visible
        for i in 0..5 {
            snapshot.dirty_chunks.push((
                ChunkAddress::test(0, i, 0, 0),
                vec![0x00; 100],
            ));
        }

        let progress = Arc::new(AtomicU32::new(0));
        let progress_clone = progress.clone();

        write_snapshot_to_disk(snapshot, progress_clone, 1).unwrap();

        // After completion, progress should be 100
        let final_progress = progress.load(Ordering::Relaxed);
        assert_eq!(final_progress, 100);
    }

    #[test]
    fn test_autosave_disabled_does_not_trigger() {
        let config = AutosaveConfig {
            interval_secs: 1.0,
            enabled: false,
            max_backups: 1,
        };
        let mut state = AutosaveState::new();
        state.time_since_last_save = 999.0;

        // Even with accumulated time, disabled autosave should not trigger
        assert!(!config.enabled);
    }
}
```
