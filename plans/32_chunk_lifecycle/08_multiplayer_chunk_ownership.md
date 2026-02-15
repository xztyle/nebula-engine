# Multiplayer Chunk Ownership

## Problem

In multiplayer, if each client independently generates chunks from the same world seed, the chunks will be identical — but only until a player modifies one. At that point, clients diverge. One player digs a tunnel, but other players see solid rock. Without a single source of truth for chunk data, the world becomes inconsistent across clients. Additionally, if two players modify the same chunk simultaneously, their changes conflict with no resolution mechanism. The server must own chunk data authoritatively: it generates chunks, applies modifications, saves dirty chunks, and distributes chunk data to all clients that need it.

## Solution

Implement a server-authoritative chunk ownership model in the `nebula_chunk` and `nebula_networking` crates. The server is the sole generator and modifier of chunk data. Clients request chunks from the server and receive the authoritative version.

### Chunk Request Protocol

```rust
use serde::{Deserialize, Serialize};
use crate::coords::ChunkAddress;

/// Messages for chunk data exchange between client and server.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ChunkMessage {
    /// Client -> Server: request chunk data at this address.
    RequestChunk {
        address: ChunkAddress,
        /// Client's known version of this chunk (0 if never seen).
        known_version: u64,
    },
    /// Server -> Client: here is the chunk data.
    ChunkData {
        address: ChunkAddress,
        /// Serialized voxel data (postcard-encoded, run-length compressed).
        data: Vec<u8>,
        /// Monotonic version number. Incremented on every server-side modification.
        version: u64,
    },
    /// Server -> Client: this chunk has not changed since your known version.
    ChunkUnchanged {
        address: ChunkAddress,
        version: u64,
    },
    /// Client -> Server: request to modify a voxel.
    ModifyVoxel {
        address: ChunkAddress,
        local_x: u8,
        local_y: u8,
        local_z: u8,
        new_voxel: VoxelType,
    },
    /// Server -> all clients: a voxel was modified (broadcast after validation).
    VoxelModified {
        address: ChunkAddress,
        local_x: u8,
        local_y: u8,
        local_z: u8,
        new_voxel: VoxelType,
        new_version: u64,
    },
}
```

### Server Chunk Manager

The server maintains the authoritative chunk cache and handles all client requests:

```rust
use bevy_ecs::prelude::*;
use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc;

/// Unique identifier for a connected client.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClientId(pub u64);

/// Server-side chunk ownership and distribution manager.
#[derive(Resource)]
pub struct ServerChunkManager {
    /// Authoritative chunk data, keyed by address.
    chunks: HashMap<ChunkAddress, ServerChunk>,
    /// Tracks which clients have requested (and received) each chunk.
    subscribers: HashMap<ChunkAddress, HashSet<ClientId>>,
    /// Pending generation requests (deduplicated).
    pending_generation: HashSet<ChunkAddress>,
}

/// Server-side chunk state.
pub struct ServerChunk {
    pub data: ChunkVoxelData,
    pub version: u64,
    pub dirty: bool,
}

impl ServerChunkManager {
    pub fn new() -> Self {
        Self {
            chunks: HashMap::new(),
            subscribers: HashMap::new(),
            pending_generation: HashSet::new(),
        }
    }

    /// Handle a chunk request from a client.
    pub fn handle_request(
        &mut self,
        client: ClientId,
        address: ChunkAddress,
        known_version: u64,
    ) -> ChunkResponse {
        // Register the client as a subscriber for this chunk
        self.subscribers
            .entry(address)
            .or_default()
            .insert(client);

        if let Some(chunk) = self.chunks.get(&address) {
            if chunk.version == known_version {
                // Client already has the latest version
                ChunkResponse::Unchanged { address, version: chunk.version }
            } else {
                // Send updated data
                let data = serialize_chunk(&chunk.data, address).unwrap();
                ChunkResponse::Data {
                    address,
                    data,
                    version: chunk.version,
                }
            }
        } else {
            // Chunk not generated yet — schedule generation (deduplicated)
            if self.pending_generation.insert(address) {
                ChunkResponse::Generating { address }
            } else {
                // Already being generated, client will be notified when ready
                ChunkResponse::Generating { address }
            }
        }
    }

    /// Called when a generation task completes on the server.
    pub fn on_chunk_generated(
        &mut self,
        address: ChunkAddress,
        data: ChunkVoxelData,
    ) -> Vec<(ClientId, ChunkAddress)> {
        self.pending_generation.remove(&address);

        let version = 1;
        self.chunks.insert(address, ServerChunk {
            data,
            version,
            dirty: false,
        });

        // Notify all waiting subscribers
        let subscribers = self.subscribers.get(&address).cloned().unwrap_or_default();
        subscribers.into_iter().map(|client| (client, address)).collect()
    }

    /// Apply a voxel modification. Only the server calls this.
    /// Returns the list of clients to notify.
    pub fn apply_modification(
        &mut self,
        address: ChunkAddress,
        x: u8, y: u8, z: u8,
        voxel: VoxelType,
    ) -> Option<(u64, Vec<ClientId>)> {
        let chunk = self.chunks.get_mut(&address)?;

        chunk.data.set(x as usize, y as usize, z as usize, voxel);
        chunk.version += 1;
        chunk.dirty = true;

        let subscribers = self.subscribers
            .get(&address)
            .cloned()
            .unwrap_or_default();

        Some((chunk.version, subscribers.into_iter().collect()))
    }

    /// Get all dirty chunks for persistence.
    pub fn dirty_chunks(&self) -> Vec<ChunkAddress> {
        self.chunks
            .iter()
            .filter(|(_, chunk)| chunk.dirty)
            .map(|(addr, _)| *addr)
            .collect()
    }

    /// Mark a chunk as saved (clean).
    pub fn mark_saved(&mut self, address: &ChunkAddress) {
        if let Some(chunk) = self.chunks.get_mut(address) {
            chunk.dirty = false;
        }
    }

    /// Check if a chunk is loaded on the server.
    pub fn has_chunk(&self, address: &ChunkAddress) -> bool {
        self.chunks.contains_key(address)
    }

    /// Get the subscriber count for a chunk (for deduplication diagnostics).
    pub fn subscriber_count(&self, address: &ChunkAddress) -> usize {
        self.subscribers.get(address).map_or(0, |s| s.len())
    }
}

#[derive(Debug)]
pub enum ChunkResponse {
    Data { address: ChunkAddress, data: Vec<u8>, version: u64 },
    Unchanged { address: ChunkAddress, version: u64 },
    Generating { address: ChunkAddress },
}
```

### Client Chunk Manager

On the client side, instead of generating chunks locally, the client requests them from the server:

```rust
/// Client-side chunk requester.
#[derive(Resource)]
pub struct ClientChunkRequester {
    /// Chunks we have requested but not yet received.
    pending_requests: HashSet<ChunkAddress>,
    /// Known versions of chunks we have received.
    known_versions: HashMap<ChunkAddress, u64>,
}

impl ClientChunkRequester {
    pub fn new() -> Self {
        Self {
            pending_requests: HashSet::new(),
            known_versions: HashMap::new(),
        }
    }

    /// Request a chunk from the server. Returns a ChunkMessage to send.
    pub fn request_chunk(&mut self, address: ChunkAddress) -> Option<ChunkMessage> {
        if self.pending_requests.contains(&address) {
            return None; // Already requested, deduplicate
        }
        self.pending_requests.insert(address);
        let known_version = self.known_versions.get(&address).copied().unwrap_or(0);
        Some(ChunkMessage::RequestChunk { address, known_version })
    }

    /// Handle a received chunk from the server.
    pub fn on_chunk_received(&mut self, address: ChunkAddress, version: u64) {
        self.pending_requests.remove(&address);
        self.known_versions.insert(address, version);
    }

    pub fn is_pending(&self, address: &ChunkAddress) -> bool {
        self.pending_requests.contains(address)
    }
}
```

### Server-Authoritative Modifications

When a client wants to modify a voxel, it sends a `ModifyVoxel` message to the server. The server validates the modification (e.g., checks that the chunk is loaded, the position is valid, the player has permission) and then applies it. The server broadcasts a `VoxelModified` message to all subscribers of that chunk, including the originating client. This ensures all clients converge on the same world state.

Clients do not apply modifications locally before server confirmation (no client-side prediction for voxel edits). This avoids rollback complexity for permanent world modifications.

### Server Persistence

The server is responsible for saving all dirty chunks. It uses the same `ChunkPersistence` system from story 07, running on the server process. Clients never write chunk data to disk in multiplayer mode.

## Outcome

The `nebula_chunk` crate exports `ChunkMessage`, `ServerChunkManager`, `ChunkResponse`, and `ClientChunkRequester`. The `nebula_networking` crate handles serialization and transport of `ChunkMessage` over the network. The server generates, caches, and distributes chunk data to clients. Modifications are server-authoritative. Duplicate chunk requests are deduplicated. Running `cargo test -p nebula_chunk` passes all multiplayer chunk ownership tests.

## Demo Integration

**Demo crate:** `nebula-demo`

In multiplayer mode, the server controls which client "owns" chunk generation for a region. Ownership transfers as players move, preventing duplicate generation.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | `Resource` derives, ECS integration |
| `tokio` | `1.49` | Async networking I/O, channel-based message passing |
| `postcard` | `1.1` | Compact serialization of chunk data for network transfer |
| `serde` | `1.0` | Derive `Serialize`/`Deserialize` for `ChunkMessage` and related types |

The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn addr(x: i64, y: i64, z: i64) -> ChunkAddress {
        ChunkAddress::new(x as i128, y as i128, z as i128)
    }

    fn client(id: u64) -> ClientId {
        ClientId(id)
    }

    /// A client requesting a chunk that the server has should receive data.
    #[test]
    fn test_client_requests_chunk_from_server() {
        let mut manager = ServerChunkManager::new();
        let chunk_addr = addr(5, 5, 5);

        // Server has a generated chunk
        let data = ChunkVoxelData::new_filled(32, VoxelType::STONE);
        manager.on_chunk_generated(chunk_addr, data);

        let response = manager.handle_request(client(1), chunk_addr, 0);
        match response {
            ChunkResponse::Data { address, version, .. } => {
                assert_eq!(address, chunk_addr);
                assert_eq!(version, 1);
            }
            _ => panic!("expected ChunkResponse::Data"),
        }
    }

    /// When the server generates a chunk, all requesting clients are notified.
    #[test]
    fn test_server_generates_and_sends_to_clients() {
        let mut manager = ServerChunkManager::new();
        let chunk_addr = addr(10, 0, 0);

        // Two clients request the same chunk before it is generated
        let r1 = manager.handle_request(client(1), chunk_addr, 0);
        let r2 = manager.handle_request(client(2), chunk_addr, 0);

        assert!(matches!(r1, ChunkResponse::Generating { .. }));
        assert!(matches!(r2, ChunkResponse::Generating { .. }));

        // Server finishes generation
        let data = ChunkVoxelData::new_filled(32, VoxelType::STONE);
        let notify = manager.on_chunk_generated(chunk_addr, data);

        // Both clients should be notified
        let notified_clients: HashSet<ClientId> = notify.iter().map(|(c, _)| *c).collect();
        assert!(notified_clients.contains(&client(1)));
        assert!(notified_clients.contains(&client(2)));
    }

    /// Duplicate requests for the same chunk should be deduplicated.
    #[test]
    fn test_duplicate_requests_are_deduplicated() {
        let mut manager = ServerChunkManager::new();
        let chunk_addr = addr(3, 3, 3);

        // Same client requests same chunk twice
        let _r1 = manager.handle_request(client(1), chunk_addr, 0);
        let _r2 = manager.handle_request(client(1), chunk_addr, 0);

        // Should only have one subscriber entry for client 1
        assert_eq!(manager.subscriber_count(&chunk_addr), 1);

        // Client-side deduplication
        let mut requester = ClientChunkRequester::new();
        let msg1 = requester.request_chunk(chunk_addr);
        let msg2 = requester.request_chunk(chunk_addr);

        assert!(msg1.is_some(), "first request should produce a message");
        assert!(msg2.is_none(), "duplicate request should be deduplicated");
    }

    /// Voxel modifications should only be applied by the server.
    #[test]
    fn test_modifications_are_server_only() {
        let mut manager = ServerChunkManager::new();
        let chunk_addr = addr(7, 7, 7);

        let data = ChunkVoxelData::new_filled(32, VoxelType::STONE);
        manager.on_chunk_generated(chunk_addr, data);

        // Subscribe a client
        manager.handle_request(client(1), chunk_addr, 0);

        // Server applies a modification
        let result = manager.apply_modification(chunk_addr, 16, 16, 16, VoxelType::AIR);
        assert!(result.is_some());

        let (new_version, notified) = result.unwrap();
        assert_eq!(new_version, 2); // version incremented from 1 to 2
        assert!(notified.contains(&client(1)));

        // Verify the chunk is now dirty
        assert!(manager.dirty_chunks().contains(&chunk_addr));
    }

    /// The server should persist all dirty chunks.
    #[test]
    fn test_server_persists_dirty_chunks() {
        let mut manager = ServerChunkManager::new();
        let addr_a = addr(1, 0, 0);
        let addr_b = addr(2, 0, 0);

        let data_a = ChunkVoxelData::new_filled(32, VoxelType::STONE);
        let data_b = ChunkVoxelData::new_filled(32, VoxelType::STONE);
        manager.on_chunk_generated(addr_a, data_a);
        manager.on_chunk_generated(addr_b, data_b);

        // Modify only chunk A
        manager.apply_modification(addr_a, 0, 0, 0, VoxelType::AIR);

        let dirty = manager.dirty_chunks();
        assert!(dirty.contains(&addr_a), "modified chunk should be dirty");
        assert!(!dirty.contains(&addr_b), "unmodified chunk should not be dirty");

        // After saving
        manager.mark_saved(&addr_a);
        let dirty_after = manager.dirty_chunks();
        assert!(!dirty_after.contains(&addr_a), "saved chunk should be clean");
    }

    /// A client that already has the latest version should receive Unchanged.
    #[test]
    fn test_client_with_latest_version_gets_unchanged() {
        let mut manager = ServerChunkManager::new();
        let chunk_addr = addr(5, 5, 5);

        let data = ChunkVoxelData::new_filled(32, VoxelType::STONE);
        manager.on_chunk_generated(chunk_addr, data);

        // First request: client has version 0, server has version 1
        let r1 = manager.handle_request(client(1), chunk_addr, 0);
        assert!(matches!(r1, ChunkResponse::Data { .. }));

        // Second request: client now has version 1
        let r2 = manager.handle_request(client(1), chunk_addr, 1);
        match r2 {
            ChunkResponse::Unchanged { version, .. } => {
                assert_eq!(version, 1);
            }
            _ => panic!("expected ChunkResponse::Unchanged for up-to-date client"),
        }
    }

    /// Modifying a non-existent chunk should return None.
    #[test]
    fn test_modify_nonexistent_chunk_returns_none() {
        let mut manager = ServerChunkManager::new();
        let result = manager.apply_modification(addr(99, 99, 99), 0, 0, 0, VoxelType::AIR);
        assert!(result.is_none());
    }
}
```
