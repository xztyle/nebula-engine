# World State Snapshots

## Problem

A multiplayer server holds the authoritative world state in memory — all modified voxel chunks, entity states, player data, and world time. If the server crashes, all unsaved progress is lost. The server needs a periodic persistence mechanism that captures the world state to disk, enabling crash recovery and orderly shutdown/restart. Full snapshots of an entire planet would be prohibitively large, so the system must support incremental snapshots that only capture changes since the last snapshot.

## Solution

### Snapshot Contents

A snapshot captures the complete recoverable state of the world:

```rust
#[derive(Serialize, Deserialize)]
pub struct WorldSnapshot {
    pub header: SnapshotHeader,
    pub modified_chunks: Vec<ChunkSnapshot>,
    pub entities: Vec<EntitySnapshot>,
    pub world_time: f64,
}

#[derive(Serialize, Deserialize)]
pub struct SnapshotHeader {
    pub version: u32,           // Format version for forward compatibility
    pub snapshot_id: u64,       // Monotonically increasing ID
    pub server_tick: u64,       // Tick at which snapshot was taken
    pub timestamp: u64,         // Wall-clock Unix millis
    pub is_incremental: bool,   // True if this only contains changes
    pub parent_snapshot_id: Option<u64>, // For incremental: which full snapshot this extends
}

#[derive(Serialize, Deserialize)]
pub struct ChunkSnapshot {
    pub chunk_id: ChunkId,
    pub voxel_data: Vec<u8>,   // Compressed voxel data
}

#[derive(Serialize, Deserialize)]
pub struct EntitySnapshot {
    pub network_id: NetworkId,
    pub components: Vec<(ComponentTypeTag, Vec<u8>)>,
}
```

### Snapshot Interval

The server takes snapshots at a configurable interval (default: every 5 minutes of real time). Snapshots are triggered by a timer system that runs within the server's tick loop:

```rust
pub struct SnapshotConfig {
    pub interval: Duration,          // default: 5 minutes
    pub incremental: bool,           // default: true
    pub snapshot_dir: PathBuf,       // default: "./snapshots/"
    pub max_snapshots_retained: usize, // default: 24 (2 hours of 5-min intervals)
}

pub struct SnapshotTimer {
    pub last_snapshot: Instant,
    pub config: SnapshotConfig,
}

impl SnapshotTimer {
    pub fn should_snapshot(&self) -> bool {
        self.last_snapshot.elapsed() >= self.config.interval
    }
}
```

### Incremental Snapshots

To minimize snapshot size and I/O cost, the system supports incremental snapshots:

- **Full snapshot**: Contains all modified chunks and all entity states. Taken periodically (e.g., every 30 minutes) or on server startup.
- **Incremental snapshot**: Contains only chunks modified since the last snapshot (full or incremental) and all entity states (entities are small relative to chunks).

The server tracks which chunks have been modified since the last snapshot using a dirty set:

```rust
pub struct DirtyChunkTracker {
    pub dirty: HashSet<ChunkId>,
}

impl DirtyChunkTracker {
    pub fn mark_dirty(&mut self, chunk_id: ChunkId) {
        self.dirty.insert(chunk_id);
    }

    pub fn drain(&mut self) -> HashSet<ChunkId> {
        std::mem::take(&mut self.dirty)
    }
}
```

### Snapshot Writing

Snapshots are written to disk asynchronously to avoid blocking the game loop. The snapshot data is serialized and compressed, then written in a background task:

```rust
pub async fn write_snapshot(
    snapshot: &WorldSnapshot,
    config: &SnapshotConfig,
) -> Result<PathBuf, SnapshotError> {
    let bytes = postcard::to_allocvec(snapshot)?;
    let compressed = lz4_flex::compress_prepend_size(&bytes);

    let filename = format!(
        "snapshot_{:06}_{}.nbsnap",
        snapshot.header.snapshot_id,
        if snapshot.header.is_incremental { "inc" } else { "full" }
    );
    let path = config.snapshot_dir.join(&filename);

    tokio::fs::write(&path, &compressed).await?;
    Ok(path)
}
```

### Snapshot Loading (Recovery)

On server startup, the recovery system:

1. Finds the latest full snapshot.
2. Applies all incremental snapshots after it in order.
3. Reconstructs the authoritative world state.

```rust
pub async fn load_world_from_snapshots(
    config: &SnapshotConfig,
) -> Result<AuthoritativeWorld, SnapshotError> {
    let snapshots = discover_snapshots(&config.snapshot_dir).await?;
    let latest_full = find_latest_full(&snapshots)?;

    let mut world = apply_full_snapshot(latest_full).await?;

    for incremental in find_incrementals_after(&snapshots, latest_full.header.snapshot_id) {
        apply_incremental_snapshot(&mut world, incremental).await?;
    }

    Ok(world)
}
```

### Version Compatibility

The `SnapshotHeader.version` field enables forward compatibility. When loading a snapshot, the version is checked:

- **Same version**: Load normally.
- **Older version**: Run migration logic (version-specific transformers).
- **Newer version**: Reject with error (cannot downgrade).

```rust
pub const CURRENT_SNAPSHOT_VERSION: u32 = 1;

pub fn check_version(header: &SnapshotHeader) -> Result<(), SnapshotError> {
    if header.version > CURRENT_SNAPSHOT_VERSION {
        return Err(SnapshotError::VersionTooNew {
            found: header.version,
            max_supported: CURRENT_SNAPSHOT_VERSION,
        });
    }
    Ok(())
}
```

## Outcome

- `nebula_multiplayer::snapshot` module containing `WorldSnapshot`, `SnapshotHeader`, `ChunkSnapshot`, `EntitySnapshot`, `SnapshotConfig`, `SnapshotTimer`, `DirtyChunkTracker`, and snapshot read/write functions.
- Periodic automatic snapshots (configurable interval, default 5 minutes).
- Incremental snapshots that only persist changed chunks.
- Crash recovery by replaying full + incremental snapshots.
- Versioned snapshot format for forward compatibility.

## Demo Integration

**Demo crate:** `nebula-demo`

The server periodically snapshots the world state. Restarting the server restores the world to the most recent snapshot. Player edits persist across server restarts.

## Crates & Dependencies

| Crate       | Version | Purpose                                         |
| ----------- | ------- | ----------------------------------------------- |
| `tokio`     | 1.49    | Async file I/O for snapshot writing/reading      |
| `serde`     | 1.0     | Serialization of snapshot structures             |
| `postcard`  | 1.1     | Binary encoding of snapshot data                 |
| `bevy_ecs`  | 0.18    | ECS world access for capturing entity state      |
| `lz4_flex`  | 0.11    | Compression of snapshot files                    |

## Unit Tests

### `test_snapshot_contains_all_modified_chunks`
Modify 10 chunks in the world. Take a full snapshot. Assert the snapshot's `modified_chunks` list contains exactly those 10 chunk IDs with correct voxel data.

### `test_incremental_snapshot_is_smaller_than_full`
Modify 100 chunks. Take a full snapshot. Modify 5 more chunks. Take an incremental snapshot. Assert the incremental snapshot's `modified_chunks` list has exactly 5 entries, and its serialized byte size is less than the full snapshot's size.

### `test_snapshot_loads_correctly`
Take a full snapshot of a world with known state (specific chunk data, entity positions). Write to disk. Load from disk. Assert the recovered world state matches the original — same voxel values, same entity positions and components.

### `test_version_mismatch_handled`
Create a snapshot header with `version = CURRENT_SNAPSHOT_VERSION + 1`. Attempt to load it. Assert the system returns `SnapshotError::VersionTooNew` and does not proceed.

### `test_snapshot_interval_is_configurable`
Set `SnapshotConfig.interval` to 1 second. Advance the timer by 500 ms. Assert `should_snapshot()` returns false. Advance by another 600 ms (total 1100 ms). Assert `should_snapshot()` returns true.
