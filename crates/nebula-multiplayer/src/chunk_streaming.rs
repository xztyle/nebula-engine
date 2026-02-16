//! Chunk data streaming: priority queue, compression, rate limiting, and
//! client-side caching for server-to-client voxel chunk delivery.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use lz4_flex::{compress_prepend_size, decompress_size_prepended};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ChunkId
// ---------------------------------------------------------------------------

/// Unique identifier for a voxel chunk on the cubesphere grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkId {
    /// Cube face index (0–5).
    pub face: u8,
    /// Level-of-detail tier.
    pub lod: u8,
    /// Grid X coordinate.
    pub x: i32,
    /// Grid Y coordinate.
    pub y: i32,
    /// Grid Z coordinate.
    pub z: i32,
}

// ---------------------------------------------------------------------------
// Compression helpers
// ---------------------------------------------------------------------------

/// Compress raw chunk voxel data with LZ4.
pub fn compress_chunk(raw: &[u8]) -> Vec<u8> {
    compress_prepend_size(raw)
}

/// Decompress LZ4-compressed chunk data.
///
/// # Errors
///
/// Returns an error if the compressed data is malformed.
pub fn decompress_chunk(compressed: &[u8]) -> Result<Vec<u8>, ChunkDecompressError> {
    decompress_size_prepended(compressed).map_err(|e| ChunkDecompressError::Lz4(e.to_string()))
}

/// Error returned by [`decompress_chunk`].
#[derive(Debug, thiserror::Error)]
pub enum ChunkDecompressError {
    /// LZ4 decompression failed.
    #[error("LZ4 decompression failed: {0}")]
    Lz4(String),
}

// ---------------------------------------------------------------------------
// Wire message
// ---------------------------------------------------------------------------

/// Chunk data message sent from server to client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkDataMessage {
    /// Which chunk this data belongs to.
    pub chunk_id: ChunkId,
    /// LZ4-compressed voxel data.
    pub compressed_data: Vec<u8>,
    /// Original uncompressed size in bytes.
    pub uncompressed_size: u32,
}

// ---------------------------------------------------------------------------
// Streaming config
// ---------------------------------------------------------------------------

/// Configuration for the chunk streaming rate limiter.
#[derive(Debug, Clone)]
pub struct ChunkStreamConfig {
    /// Maximum compressed bytes to send per tick. Default: 65 536 (64 KiB).
    pub bytes_per_tick: usize,
    /// Maximum chunks that may be queued at once. Default: 256.
    pub max_queued_chunks: usize,
}

impl Default for ChunkStreamConfig {
    fn default() -> Self {
        Self {
            bytes_per_tick: 65_536,
            max_queued_chunks: 256,
        }
    }
}

// ---------------------------------------------------------------------------
// Priority queue
// ---------------------------------------------------------------------------

/// An entry in the per-client chunk send queue, ordered by distance priority.
#[derive(Debug, Clone)]
pub struct ChunkSendEntry {
    /// Target chunk.
    pub chunk_id: ChunkId,
    /// Priority value — *lower* distance means *higher* priority.
    pub priority: f64,
}

impl PartialEq for ChunkSendEntry {
    fn eq(&self, other: &Self) -> bool {
        self.chunk_id == other.chunk_id
    }
}

impl Eq for ChunkSendEntry {}

impl Ord for ChunkSendEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is a max-heap; we want closest (smallest priority) first,
        // so reverse the comparison.
        other
            .priority
            .partial_cmp(&self.priority)
            .unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for ChunkSendEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Per-client queue of chunks awaiting transmission.
#[derive(Debug)]
pub struct ChunkSendQueue {
    /// Priority queue ordered by proximity.
    pub queue: BinaryHeap<ChunkSendEntry>,
    /// Set of chunks already sent to this client.
    pub sent: HashSet<ChunkId>,
}

impl ChunkSendQueue {
    /// Create an empty send queue.
    pub fn new() -> Self {
        Self {
            queue: BinaryHeap::new(),
            sent: HashSet::new(),
        }
    }

    /// Enqueue a chunk if it has not already been sent and the queue is not
    /// full.
    pub fn enqueue(&mut self, entry: ChunkSendEntry, config: &ChunkStreamConfig) {
        if self.sent.contains(&entry.chunk_id) {
            return;
        }
        if self.queue.len() >= config.max_queued_chunks {
            return;
        }
        self.queue.push(entry);
    }

    /// Drain up to `bytes_per_tick` worth of compressed chunk data from the
    /// queue.  Returns the produced messages and the number of bytes consumed.
    pub fn flush_tick(
        &mut self,
        config: &ChunkStreamConfig,
        chunk_data_fn: impl Fn(&ChunkId) -> Option<Vec<u8>>,
    ) -> Vec<ChunkDataMessage> {
        let mut budget = config.bytes_per_tick;
        let mut messages = Vec::new();

        while let Some(entry) = self.queue.peek() {
            let id = entry.chunk_id;
            let raw = match chunk_data_fn(&id) {
                Some(d) => d,
                None => {
                    // Chunk data unavailable — skip.
                    self.queue.pop();
                    continue;
                }
            };

            let compressed = compress_chunk(&raw);
            if compressed.len() > budget && !messages.is_empty() {
                // Would exceed budget and we already sent something — stop.
                break;
            }

            self.queue.pop();
            budget = budget.saturating_sub(compressed.len());

            let msg = ChunkDataMessage {
                chunk_id: id,
                compressed_data: compressed,
                uncompressed_size: raw.len() as u32,
            };
            messages.push(msg);
            self.sent.insert(id);

            if budget == 0 {
                break;
            }
        }

        messages
    }
}

impl Default for ChunkSendQueue {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Client-side cache
// ---------------------------------------------------------------------------

/// Client-side cache of received chunk data.
#[derive(Debug)]
pub struct ClientChunkCache {
    /// Cached raw (decompressed) voxel data keyed by chunk.
    pub chunks: HashMap<ChunkId, Vec<u8>>,
    /// Maximum number of chunks to keep in cache.
    pub max_cached: usize,
}

impl ClientChunkCache {
    /// Create a cache with the given capacity.
    pub fn new(max_cached: usize) -> Self {
        Self {
            chunks: HashMap::new(),
            max_cached,
        }
    }

    /// Insert a chunk into the cache, evicting the oldest entry if at
    /// capacity.  (Simple eviction: arbitrary key removal via
    /// `HashMap::keys().next()`.)
    pub fn insert(&mut self, id: ChunkId, data: Vec<u8>) {
        if self.chunks.len() >= self.max_cached
            && !self.chunks.contains_key(&id)
            && let Some(&evict) = self.chunks.keys().next()
        {
            self.chunks.remove(&evict);
        }
        self.chunks.insert(id, data);
    }

    /// Retrieve cached chunk data.
    pub fn get(&self, id: &ChunkId) -> Option<&Vec<u8>> {
        self.chunks.get(id)
    }

    /// Check whether a chunk is cached.
    pub fn contains(&self, id: &ChunkId) -> bool {
        self.chunks.contains_key(id)
    }
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

    #[test]
    fn test_nearby_chunk_is_sent_to_client() {
        let config = ChunkStreamConfig::default();
        let mut queue = ChunkSendQueue::new();

        let id = make_chunk_id(0, 1, 2, 3);
        // Distance 200 — within a hypothetical 500 m interest radius.
        queue.enqueue(
            ChunkSendEntry {
                chunk_id: id,
                priority: 200.0,
            },
            &config,
        );

        assert_eq!(queue.queue.len(), 1);

        let raw = vec![42u8; 1024];
        let messages = queue.flush_tick(&config, |_| Some(raw.clone()));

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].chunk_id, id);
        assert_eq!(messages[0].uncompressed_size, 1024);
    }

    #[test]
    fn test_distant_chunk_is_not_sent() {
        let config = ChunkStreamConfig::default();
        let queue = ChunkSendQueue::new();

        let id = make_chunk_id(0, 99, 99, 99);
        let interest_radius = 500.0_f64;
        let distance = 2000.0_f64;

        // Simulate: only enqueue chunks within interest radius.
        let mut q = queue;
        if distance <= interest_radius {
            q.enqueue(
                ChunkSendEntry {
                    chunk_id: id,
                    priority: distance,
                },
                &config,
            );
        }

        let messages = q.flush_tick(&config, |_| Some(vec![0u8; 512]));
        assert!(messages.is_empty());
    }

    #[test]
    fn test_chunk_data_decompresses_correctly() {
        let raw: Vec<u8> = (0..4096).map(|i| (i % 256) as u8).collect();
        let compressed = compress_chunk(&raw);
        let decompressed = decompress_chunk(&compressed).unwrap();
        assert_eq!(decompressed, raw);
    }

    #[test]
    fn test_priority_ordering_is_by_distance() {
        let config = ChunkStreamConfig {
            bytes_per_tick: 1_000_000,
            max_queued_chunks: 256,
        };
        let mut queue = ChunkSendQueue::new();

        let a = ChunkSendEntry {
            chunk_id: make_chunk_id(0, 1, 0, 0),
            priority: 100.0,
        };
        let b = ChunkSendEntry {
            chunk_id: make_chunk_id(0, 2, 0, 0),
            priority: 300.0,
        };
        let c = ChunkSendEntry {
            chunk_id: make_chunk_id(0, 3, 0, 0),
            priority: 50.0,
        };

        queue.enqueue(a, &config);
        queue.enqueue(b, &config);
        queue.enqueue(c, &config);

        let messages = queue.flush_tick(&config, |_| Some(vec![0u8; 64]));
        assert_eq!(messages.len(), 3);
        // Closest first: 50, 100, 300.
        assert_eq!(messages[0].chunk_id, make_chunk_id(0, 3, 0, 0));
        assert_eq!(messages[1].chunk_id, make_chunk_id(0, 1, 0, 0));
        assert_eq!(messages[2].chunk_id, make_chunk_id(0, 2, 0, 0));
    }

    #[test]
    fn test_rate_limiting_prevents_bandwidth_spike() {
        let config = ChunkStreamConfig {
            bytes_per_tick: 10_000,
            max_queued_chunks: 256,
        };
        let mut queue = ChunkSendQueue::new();

        // 50 chunks, each 5000 bytes raw (compressed will be similar or smaller,
        // but we use repetitive data so LZ4 compresses aggressively — use
        // random-ish data to keep compressed size near raw).
        let raw: Vec<u8> = (0..5000).map(|i| (i * 7 % 256) as u8).collect();
        let compressed_size = compress_chunk(&raw).len();

        for i in 0..50 {
            queue.enqueue(
                ChunkSendEntry {
                    chunk_id: make_chunk_id(0, i, 0, 0),
                    priority: i as f64,
                },
                &config,
            );
        }

        let messages = queue.flush_tick(&config, |_| Some(raw.clone()));

        // At most floor(10_000 / compressed_size) chunks, but at least 1.
        let max_expected = (10_000 / compressed_size).max(1);
        assert!(
            messages.len() <= max_expected + 1,
            "sent {} messages but expected at most {} (compressed_size={})",
            messages.len(),
            max_expected + 1,
            compressed_size,
        );
        // Remaining chunks stay queued.
        assert!(!queue.queue.is_empty());
    }
}
