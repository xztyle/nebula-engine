# Chunk Data Streaming

## Problem

A cubesphere-voxel planet contains an enormous amount of voxel data — far too much to send to a client all at once. As a player explores the world, the client needs voxel chunk data for nearby regions to render terrain and support interaction. The server must stream chunk data to clients on demand, compressed for bandwidth efficiency, prioritized by proximity, and rate-limited to avoid saturating the TCP connection.

## Solution

### Chunk Interest

The spatial interest system (Story 03) determines which chunks fall within a client's interest area. When the interest system detects that a client's interest area overlaps chunks the client has not yet received, those chunks are queued for streaming.

Each chunk is identified by its `ChunkId` (cube face, LOD level, and grid coordinates from Epic 06):

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkId {
    pub face: CubeFace,
    pub lod: u8,
    pub x: i32,
    pub y: i32,
    pub z: i32,
}
```

### Streaming Pipeline

The server maintains a per-client **chunk send queue** — a priority queue of chunks awaiting transmission.

```rust
pub struct ChunkSendQueue {
    pub queue: BinaryHeap<ChunkSendEntry>,
    pub sent: HashSet<ChunkId>,
}

pub struct ChunkSendEntry {
    pub chunk_id: ChunkId,
    pub priority: f64, // lower distance = higher priority
}

impl Ord for ChunkSendEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.priority.partial_cmp(&self.priority).unwrap_or(Ordering::Equal)
    }
}
```

Each tick, the streaming system:

1. Checks the client's interest area for chunks not yet in the `sent` set.
2. Inserts missing chunks into the priority queue with priority based on distance to the client.
3. Pops the highest-priority chunk (closest to client).
4. Serializes and compresses the chunk data.
5. Sends up to the rate limit for this tick.

### Compression

Chunk voxel data is compressed with LZ4 before transmission. LZ4 is chosen for its extremely fast compression and decompression speeds, which is critical for real-time streaming. Voxel data compresses well because large regions of a chunk often contain the same material (air, stone, dirt).

```rust
use lz4_flex::{compress_prepend_size, decompress_size_prepended};

pub fn compress_chunk(raw: &[u8]) -> Vec<u8> {
    compress_prepend_size(raw)
}

pub fn decompress_chunk(compressed: &[u8]) -> Result<Vec<u8>, DecompressError> {
    decompress_size_prepended(compressed)
}
```

### Wire Message

```rust
#[derive(Serialize, Deserialize)]
pub struct ChunkDataMessage {
    pub chunk_id: ChunkId,
    pub compressed_data: Vec<u8>,
    pub uncompressed_size: u32,
}
```

### Rate Limiting

The streaming system enforces a per-client per-tick byte budget for chunk data (configurable, default: 64 KB/tick at 60 Hz = ~3.75 MB/s). This prevents chunk streaming from monopolizing the connection and starving entity replication or chat messages.

```rust
pub struct ChunkStreamConfig {
    pub bytes_per_tick: usize,       // default: 65536 (64 KB)
    pub max_queued_chunks: usize,    // default: 256
}
```

When the byte budget is exhausted for a tick, remaining chunks stay in the queue for the next tick.

### Client-Side Caching

The client caches received chunks in memory and optionally on disk. When a chunk leaves the interest area but has not been modified, it remains in cache. If the client re-enters the area later, it can use the cached version without re-requesting from the server. The server tracks which chunks each client has (via the `sent` set) and only sends chunks the client has not received or that have been modified since last sent.

```rust
pub struct ClientChunkCache {
    pub chunks: HashMap<ChunkId, ChunkData>,
    pub max_cached: usize,
}
```

### Priority by Distance

Distance is computed from the client's position to the chunk center using the 128-bit coordinate system. Chunks directly in front of the player (based on view direction) receive a small priority boost to reduce pop-in in the direction of travel.

## Outcome

- `nebula_multiplayer::chunk_streaming` module containing `ChunkSendQueue`, `ChunkSendEntry`, `ChunkDataMessage`, `ChunkStreamConfig`, and `ClientChunkCache`.
- LZ4 compression of chunk data for bandwidth efficiency.
- Distance-based priority queue ensuring nearby terrain loads first.
- Per-tick rate limiting preventing bandwidth spikes.
- Client-side chunk cache with configurable eviction.

## Demo Integration

**Demo crate:** `nebula-demo`

Terrain chunks are streamed from the server to the client as the player moves. The client receives authoritative chunk data rather than generating terrain locally.

## Crates & Dependencies

| Crate        | Version | Purpose                                       |
| ------------ | ------- | --------------------------------------------- |
| `tokio`      | 1.49    | Async TCP for streaming chunk data             |
| `serde`      | 1.0     | Serialization of chunk messages                |
| `postcard`   | 1.1     | Binary encoding of chunk data messages         |
| `bevy_ecs`   | 0.18    | ECS integration for chunk components           |
| `lz4_flex`   | 0.11    | Fast LZ4 compression/decompression             |

## Unit Tests

### `test_nearby_chunk_is_sent_to_client`
Place a client with interest radius 500 m. A chunk center is 200 m away. Run the streaming system. Assert the chunk appears in the send queue and a `ChunkDataMessage` is produced for it.

### `test_distant_chunk_is_not_sent`
Place a chunk 2000 m from the client (interest radius 500 m). Run the streaming system. Assert no `ChunkDataMessage` is produced for that chunk.

### `test_chunk_data_decompresses_correctly`
Create a `ChunkData` with known voxel values. Compress with `compress_chunk`. Decompress with `decompress_chunk`. Assert the decompressed bytes match the original byte-for-byte.

### `test_priority_ordering_is_by_distance`
Queue three chunks at distances 100 m, 300 m, and 50 m. Pop from the priority queue. Assert the order is: 50 m, 100 m, 300 m (closest first).

### `test_rate_limiting_prevents_bandwidth_spike`
Set `bytes_per_tick` to 10,000 bytes. Queue 50 chunks of 5,000 bytes each. Run one tick. Assert that at most 2 chunks are sent (10,000 / 5,000 = 2) and the remaining 48 stay queued.
