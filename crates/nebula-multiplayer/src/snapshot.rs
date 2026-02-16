//! World state snapshots: periodic persistence of server-authoritative world
//! state for crash recovery and orderly restarts.
//!
//! Supports full and incremental snapshots. Incremental snapshots only capture
//! chunks modified since the last snapshot, minimizing I/O cost.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::chunk_streaming::ChunkId;
use crate::replication::{ComponentTypeTag, NetworkId};

// ---------------------------------------------------------------------------
// Snapshot version
// ---------------------------------------------------------------------------

/// Current snapshot format version.
pub const CURRENT_SNAPSHOT_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Snapshot structures
// ---------------------------------------------------------------------------

/// Complete recoverable world state captured at a single server tick.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WorldSnapshot {
    /// Metadata about this snapshot.
    pub header: SnapshotHeader,
    /// Modified voxel chunks (all for full, dirty-only for incremental).
    pub modified_chunks: Vec<ChunkSnapshot>,
    /// All replicated entity states.
    pub entities: Vec<EntitySnapshot>,
    /// In-game world time.
    pub world_time: f64,
}

/// Snapshot metadata header.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SnapshotHeader {
    /// Format version for forward compatibility.
    pub version: u32,
    /// Monotonically increasing snapshot identifier.
    pub snapshot_id: u64,
    /// Server tick at which the snapshot was taken.
    pub server_tick: u64,
    /// Wall-clock Unix milliseconds.
    pub timestamp: u64,
    /// `true` if this snapshot only contains changes since the parent.
    pub is_incremental: bool,
    /// For incremental snapshots: the full snapshot this extends.
    pub parent_snapshot_id: Option<u64>,
}

/// A single chunk's voxel data within a snapshot.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChunkSnapshot {
    /// Which chunk this data belongs to.
    pub chunk_id: ChunkId,
    /// Compressed voxel data.
    pub voxel_data: Vec<u8>,
}

/// A single entity's replicated state within a snapshot.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EntitySnapshot {
    /// The entity's network identifier.
    pub network_id: NetworkId,
    /// All replicated components as (tag, bytes) pairs.
    pub components: Vec<(ComponentTypeTag, Vec<u8>)>,
}

// ---------------------------------------------------------------------------
// Configuration & timer
// ---------------------------------------------------------------------------

/// Configuration for snapshot persistence.
#[derive(Debug, Clone)]
pub struct SnapshotConfig {
    /// Interval between automatic snapshots. Default: 5 minutes.
    pub interval: Duration,
    /// Whether to use incremental snapshots. Default: `true`.
    pub incremental: bool,
    /// Directory to store snapshot files.
    pub snapshot_dir: PathBuf,
    /// Maximum number of snapshot files to retain. Default: 24.
    pub max_snapshots_retained: usize,
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(300),
            incremental: true,
            snapshot_dir: PathBuf::from("./snapshots/"),
            max_snapshots_retained: 24,
        }
    }
}

/// Timer that tracks when the next snapshot should be taken.
#[derive(Debug)]
pub struct SnapshotTimer {
    /// When the last snapshot was taken.
    pub last_snapshot: Instant,
    /// Snapshot configuration.
    pub config: SnapshotConfig,
}

impl SnapshotTimer {
    /// Creates a new timer that considers a snapshot as just taken.
    pub fn new(config: SnapshotConfig) -> Self {
        Self {
            last_snapshot: Instant::now(),
            config,
        }
    }

    /// Returns `true` if enough time has elapsed for a new snapshot.
    pub fn should_snapshot(&self) -> bool {
        self.last_snapshot.elapsed() >= self.config.interval
    }

    /// Resets the timer to now (call after taking a snapshot).
    pub fn reset(&mut self) {
        self.last_snapshot = Instant::now();
    }
}

// ---------------------------------------------------------------------------
// Dirty chunk tracker
// ---------------------------------------------------------------------------

/// Tracks chunks modified since the last snapshot for incremental writes.
#[derive(Debug, Default)]
pub struct DirtyChunkTracker {
    dirty: HashSet<ChunkId>,
}

impl DirtyChunkTracker {
    /// Creates an empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks a chunk as dirty (modified since last snapshot).
    pub fn mark_dirty(&mut self, chunk_id: ChunkId) {
        self.dirty.insert(chunk_id);
    }

    /// Drains all dirty chunk IDs, resetting the set.
    pub fn drain(&mut self) -> HashSet<ChunkId> {
        std::mem::take(&mut self.dirty)
    }

    /// Returns the number of currently dirty chunks.
    pub fn len(&self) -> usize {
        self.dirty.len()
    }

    /// Returns `true` if no chunks are dirty.
    pub fn is_empty(&self) -> bool {
        self.dirty.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during snapshot operations.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    /// Snapshot file version is newer than supported.
    #[error("snapshot version {found} is newer than max supported {max_supported}")]
    VersionTooNew {
        /// Version found in the file.
        found: u32,
        /// Maximum version this build supports.
        max_supported: u32,
    },
    /// I/O error reading or writing snapshot files.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Serialization / deserialization error.
    #[error("serialization error: {0}")]
    Serialization(String),
}

// ---------------------------------------------------------------------------
// Write / Load
// ---------------------------------------------------------------------------

/// Writes a snapshot to disk (synchronous). Returns the path written.
///
/// # Errors
///
/// Returns [`SnapshotError`] on I/O or serialization failure.
pub fn write_snapshot(
    snapshot: &WorldSnapshot,
    config: &SnapshotConfig,
) -> Result<PathBuf, SnapshotError> {
    let bytes =
        postcard::to_allocvec(snapshot).map_err(|e| SnapshotError::Serialization(e.to_string()))?;
    let compressed = lz4_flex::compress_prepend_size(&bytes);

    let kind = if snapshot.header.is_incremental {
        "inc"
    } else {
        "full"
    };
    let filename = format!(
        "snapshot_{:06}_{}.nbsnap",
        snapshot.header.snapshot_id, kind
    );
    let path = config.snapshot_dir.join(&filename);

    std::fs::create_dir_all(&config.snapshot_dir)?;
    std::fs::write(&path, &compressed)?;
    Ok(path)
}

/// Loads a snapshot from a file on disk.
///
/// # Errors
///
/// Returns [`SnapshotError`] on I/O, decompression, deserialization, or
/// version mismatch.
pub fn load_snapshot(path: &Path) -> Result<WorldSnapshot, SnapshotError> {
    let compressed = std::fs::read(path)?;
    let bytes = lz4_flex::decompress_size_prepended(&compressed)
        .map_err(|e| SnapshotError::Serialization(e.to_string()))?;
    let snapshot: WorldSnapshot =
        postcard::from_bytes(&bytes).map_err(|e| SnapshotError::Serialization(e.to_string()))?;
    check_version(&snapshot.header)?;
    Ok(snapshot)
}

/// Validates that the snapshot version is supported.
///
/// # Errors
///
/// Returns [`SnapshotError::VersionTooNew`] if the header version exceeds
/// [`CURRENT_SNAPSHOT_VERSION`].
pub fn check_version(header: &SnapshotHeader) -> Result<(), SnapshotError> {
    if header.version > CURRENT_SNAPSHOT_VERSION {
        return Err(SnapshotError::VersionTooNew {
            found: header.version,
            max_supported: CURRENT_SNAPSHOT_VERSION,
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chunk_id(face: u8, x: i32, y: i32, z: i32) -> ChunkId {
        ChunkId {
            face,
            lod: 0,
            x,
            y,
            z,
        }
    }

    fn make_chunk_snapshot(face: u8, x: i32, y: i32, z: i32, data: &[u8]) -> ChunkSnapshot {
        ChunkSnapshot {
            chunk_id: make_chunk_id(face, x, y, z),
            voxel_data: data.to_vec(),
        }
    }

    fn make_entity(id: u64) -> EntitySnapshot {
        EntitySnapshot {
            network_id: NetworkId(id),
            components: vec![("Position".to_string(), vec![1, 2, 3])],
        }
    }

    fn make_full_snapshot(id: u64, chunks: Vec<ChunkSnapshot>) -> WorldSnapshot {
        WorldSnapshot {
            header: SnapshotHeader {
                version: CURRENT_SNAPSHOT_VERSION,
                snapshot_id: id,
                server_tick: id * 100,
                timestamp: 1_000_000 + id,
                is_incremental: false,
                parent_snapshot_id: None,
            },
            modified_chunks: chunks,
            entities: vec![make_entity(1), make_entity(2)],
            world_time: 42.0,
        }
    }

    #[test]
    fn test_snapshot_contains_all_modified_chunks() {
        let chunks: Vec<ChunkSnapshot> = (0..10)
            .map(|i| make_chunk_snapshot(0, i, 0, 0, &[i as u8; 64]))
            .collect();
        let snapshot = make_full_snapshot(1, chunks.clone());

        assert_eq!(snapshot.modified_chunks.len(), 10);
        for (i, cs) in snapshot.modified_chunks.iter().enumerate() {
            assert_eq!(cs.chunk_id, make_chunk_id(0, i as i32, 0, 0));
            assert_eq!(cs.voxel_data, vec![i as u8; 64]);
        }
    }

    #[test]
    fn test_incremental_snapshot_is_smaller_than_full() {
        // Full snapshot with 100 chunks.
        let full_chunks: Vec<ChunkSnapshot> = (0..100)
            .map(|i| make_chunk_snapshot(0, i, 0, 0, &[i as u8; 256]))
            .collect();
        let full = make_full_snapshot(1, full_chunks);
        let full_bytes = postcard::to_allocvec(&full).unwrap();

        // Incremental with only 5 chunks.
        let inc_chunks: Vec<ChunkSnapshot> = (100..105)
            .map(|i| make_chunk_snapshot(0, i, 0, 0, &[i as u8; 256]))
            .collect();
        let inc = WorldSnapshot {
            header: SnapshotHeader {
                version: CURRENT_SNAPSHOT_VERSION,
                snapshot_id: 2,
                server_tick: 200,
                timestamp: 1_000_002,
                is_incremental: true,
                parent_snapshot_id: Some(1),
            },
            modified_chunks: inc_chunks,
            entities: vec![make_entity(1), make_entity(2)],
            world_time: 43.0,
        };
        let inc_bytes = postcard::to_allocvec(&inc).unwrap();

        assert_eq!(inc.modified_chunks.len(), 5);
        assert!(
            inc_bytes.len() < full_bytes.len(),
            "incremental {} should be smaller than full {}",
            inc_bytes.len(),
            full_bytes.len()
        );
    }

    #[test]
    fn test_snapshot_loads_correctly() {
        let dir = std::env::temp_dir().join("nebula_snap_test_load");
        let _ = std::fs::remove_dir_all(&dir);

        let config = SnapshotConfig {
            snapshot_dir: dir.clone(),
            ..Default::default()
        };

        let chunks = vec![
            make_chunk_snapshot(0, 1, 2, 3, &[0xAB; 128]),
            make_chunk_snapshot(1, 4, 5, 6, &[0xCD; 128]),
        ];
        let original = make_full_snapshot(1, chunks);

        let path = write_snapshot(&original, &config).unwrap();
        let loaded = load_snapshot(&path).unwrap();

        assert_eq!(loaded.header.snapshot_id, original.header.snapshot_id);
        assert_eq!(loaded.modified_chunks.len(), 2);
        assert_eq!(loaded.modified_chunks[0].voxel_data, vec![0xAB; 128]);
        assert_eq!(loaded.modified_chunks[1].voxel_data, vec![0xCD; 128]);
        assert_eq!(loaded.entities.len(), 2);
        assert!((loaded.world_time - 42.0).abs() < f64::EPSILON);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_version_mismatch_handled() {
        let header = SnapshotHeader {
            version: CURRENT_SNAPSHOT_VERSION + 1,
            snapshot_id: 1,
            server_tick: 100,
            timestamp: 1_000_000,
            is_incremental: false,
            parent_snapshot_id: None,
        };
        let result = check_version(&header);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, SnapshotError::VersionTooNew { found, max_supported }
                if found == CURRENT_SNAPSHOT_VERSION + 1
                && max_supported == CURRENT_SNAPSHOT_VERSION
            ),
            "expected VersionTooNew, got: {err:?}"
        );
    }

    #[test]
    fn test_snapshot_interval_is_configurable() {
        let config = SnapshotConfig {
            interval: Duration::from_millis(1000),
            ..Default::default()
        };
        let timer = SnapshotTimer {
            last_snapshot: Instant::now(),
            config,
        };

        // Just created — should not trigger yet.
        assert!(!timer.should_snapshot());

        // Simulate elapsed time by creating a timer with a past instant.
        let timer_old = SnapshotTimer {
            last_snapshot: Instant::now() - Duration::from_millis(1100),
            config: SnapshotConfig {
                interval: Duration::from_millis(1000),
                ..Default::default()
            },
        };
        assert!(timer_old.should_snapshot());
    }
}
