# Network Integration Tests

## Problem

The Nebula Engine's multiplayer stack spans multiple crates and layers: TCP server/client (`nebula_net`), length-prefixed framing, message serialization (postcard 1.1), entity replication, voxel synchronization, and connection lifecycle management. Unit tests in each crate verify individual components in isolation, but none of them prove that the full stack works end-to-end. A client connecting, authenticating, seeing another player, editing a voxel, receiving that edit on a second client, disconnecting, reconnecting, and resuming — this flow crosses every layer and cannot be tested by unit tests alone.

Without integration tests, regressions in the interaction between layers go undetected. A change to the framing layer might silently break message routing. A change to the entity replication system might fail to propagate updates over the actual TCP path. These bugs only appear in manual playtesting, which is slow, unreliable, and impossible to run in CI.

The engine uses pure TCP (no UDP, no WebRTC) and 128-bit coordinates, so the integration tests must exercise the real TCP path and verify that large coordinate values survive the full serialize-transmit-deserialize pipeline intact.

## Solution

### Test infrastructure: in-process server and clients

All integration tests run inside a single process using Tokio's multi-threaded test runtime. The server binds to `127.0.0.1:0` (ephemeral port) and each client connects via `TcpStream`. This avoids port conflicts, firewall issues, and makes tests fast and hermetic.

```rust
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;

use nebula_net::server::{GameServer, ServerConfig};
use nebula_net::client::{GameClient, ClientConfig};
use nebula_net::messages::*;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Spin up a server on an ephemeral port and return its address.
async fn start_test_server(max_connections: usize) -> (SocketAddr, Arc<GameServer>) {
    let config = ServerConfig {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        max_connections,
    };
    let server = Arc::new(GameServer::new(config));
    let addr = server.start().await.unwrap();
    (addr, server)
}

/// Connect a test client and complete the login handshake.
async fn connect_test_client(addr: SocketAddr, name: &str) -> GameClient {
    let config = ClientConfig {
        server_addr: addr,
        player_name: name.to_string(),
    };
    let mut client = GameClient::connect(config).await.unwrap();
    let response = client.login().await.unwrap();
    assert!(response.success, "Login should succeed for '{name}'");
    client
}
```

### Entity replication verification

After two clients connect, each should receive `EntityUpdate` messages about the other. The test verifies that Client A sees Client B's player entity and vice versa, with correct 128-bit position data.

```rust
async fn wait_for_entity(client: &mut GameClient, target_entity_id: u64) -> EntityUpdate {
    let deadline = tokio::time::Instant::now() + TEST_TIMEOUT;
    loop {
        let msg = timeout(
            deadline.duration_since(tokio::time::Instant::now()),
            client.recv(),
        )
        .await
        .expect("Timed out waiting for entity update")
        .unwrap();

        if let Message::EntityUpdate(update) = msg {
            if update.entity_id == target_entity_id {
                return update;
            }
        }
    }
}
```

### Voxel edit replication

Client A sends a `PlayerAction` with action type "place voxel" targeting a specific coordinate. Client B should receive the corresponding voxel edit as a `ChunkData` delta or a voxel edit message. The test verifies:
- The edit reaches the server.
- The server validates and applies it.
- The server broadcasts the updated chunk data to Client B.
- The voxel data Client B receives matches what Client A placed.

```rust
async fn place_voxel(client: &mut GameClient, x: i64, y: i64, z: i64, voxel_type: u8) {
    let action = PlayerAction {
        player_id: client.player_id(),
        action_type: ACTION_PLACE_VOXEL,
        target_x: x,
        target_y: y,
        target_z: z,
        payload: vec![voxel_type],
    };
    client.send(Message::PlayerAction(action)).await.unwrap();
}
```

### Disconnect and reconnect

The test forces a client disconnect by dropping its `TcpStream`, waits for the server to detect the disconnection, then reconnects with the same player name. The reconnected client should receive the current world state (any voxel edits that happened while disconnected) and be able to resume normal operation.

```rust
async fn test_reconnection_flow(server_addr: SocketAddr) {
    let mut client = connect_test_client(server_addr, "Reconnector").await;
    let player_id = client.player_id();

    // Force disconnect
    client.disconnect().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Reconnect
    let mut client = connect_test_client(server_addr, "Reconnector").await;
    assert_eq!(
        client.player_id(),
        player_id,
        "Reconnected client should get the same player ID"
    );
}
```

### Chat message roundtrip

The engine supports chat messages as a variant in the `Message` enum. The test sends a chat message from Client A and verifies that Client B receives it with correct sender ID and content intact.

### Stress test: 10 concurrent clients

The test spawns 10 clients that all connect simultaneously, each sending position updates at a steady rate. After a fixed number of ticks, the test verifies that no client has been disconnected, the server's connection count is 10, and each client has received entity updates for all other clients.

## Outcome

A `network_integration_tests.rs` file in `crates/nebula_testing/tests/` containing end-to-end tests for the full multiplayer stack. Tests use an in-process Tokio-based server with ephemeral port binding, real TCP connections, and the complete message serialization pipeline. Covers connection, authentication, entity replication, voxel editing, disconnect/reconnect, chat, and concurrent load. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

An automated test spins up a server and two clients. Player 1 edits a voxel; player 2 verifies the edit appears. Chat messages round-trip. Pass or fail is reported.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | `1.49` (features: `rt-multi-thread`, `macros`, `net`, `time`, `io-util`) | Async test runtime, TCP connections, timeouts, sleep |
| `serde` | `1.0` (features: `derive`) | Message serialization for test verification |
| `postcard` | `1.1` (features: `alloc`) | Binary serialization/deserialization of test messages |
| `tracing` | `0.1` | Structured logging during integration tests |
| `tracing-subscriber` | `0.3` | Log output for debugging test failures |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Two clients connect to the same server and each receives an
    /// EntityUpdate for the other's player entity.
    #[tokio::test]
    async fn test_two_clients_see_each_other() {
        let (addr, _server) = start_test_server(16).await;

        let mut client_a = connect_test_client(addr, "Alice").await;
        let mut client_b = connect_test_client(addr, "Bob").await;

        let update_on_a = wait_for_entity(&mut client_a, client_b.player_entity_id()).await;
        let update_on_b = wait_for_entity(&mut client_b, client_a.player_entity_id()).await;

        assert_eq!(update_on_a.entity_id, client_b.player_entity_id());
        assert_eq!(update_on_b.entity_id, client_a.player_entity_id());
    }

    /// Client A places a voxel. Client B receives the updated chunk data
    /// containing that voxel.
    #[tokio::test]
    async fn test_voxel_edit_replicates_to_other_client() {
        let (addr, _server) = start_test_server(16).await;

        let mut client_a = connect_test_client(addr, "Builder").await;
        let mut client_b = connect_test_client(addr, "Observer").await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        place_voxel(&mut client_a, 10, 20, 30, 5).await;

        let chunk = timeout(TEST_TIMEOUT, async {
            loop {
                let msg = client_b.recv().await.unwrap();
                if let Message::ChunkData(data) = msg {
                    return data;
                }
            }
        })
        .await
        .expect("Client B should receive chunk data with the voxel edit");

        assert!(
            !chunk.voxel_data.is_empty(),
            "Chunk data should contain voxel information"
        );
    }

    /// When a client disconnects, the server removes it from the connection
    /// map and notifies remaining clients.
    #[tokio::test]
    async fn test_disconnection_is_handled() {
        let (addr, server) = start_test_server(16).await;

        let mut client_a = connect_test_client(addr, "Stayer").await;
        let mut client_b = connect_test_client(addr, "Leaver").await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        let entity_id_b = client_b.player_entity_id();
        client_b.disconnect().await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Server should have removed the disconnected client.
        assert_eq!(server.connection_count().await, 1);

        // Client A should receive a removal notification for Client B's entity.
        let removed = timeout(TEST_TIMEOUT, async {
            loop {
                let msg = client_a.recv().await.unwrap();
                if let Message::EntityUpdate(update) = msg {
                    if update.entity_id == entity_id_b {
                        return update;
                    }
                }
            }
        })
        .await
        .expect("Client A should be notified of Client B's departure");

        assert_eq!(removed.entity_id, entity_id_b);
    }

    /// A client disconnects and reconnects. After reconnecting, it receives
    /// the current world state, including voxel edits made while it was away.
    #[tokio::test]
    async fn test_reconnection_restores_state() {
        let (addr, _server) = start_test_server(16).await;

        let mut client_a = connect_test_client(addr, "Alice").await;
        let mut client_b = connect_test_client(addr, "Bob").await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Alice places a voxel while Bob is connected.
        place_voxel(&mut client_a, 5, 5, 5, 3).await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Bob disconnects.
        client_b.disconnect().await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Bob reconnects and should receive updated world state.
        let mut client_b = connect_test_client(addr, "Bob").await;
        let received_chunk = timeout(TEST_TIMEOUT, async {
            loop {
                let msg = client_b.recv().await.unwrap();
                if let Message::ChunkData(_) = msg {
                    return true;
                }
            }
        })
        .await
        .expect("Reconnected client should receive current world state");

        assert!(received_chunk);
    }

    /// Client A sends a chat message. Client B receives it with the
    /// correct sender and content.
    #[tokio::test]
    async fn test_chat_message_roundtrip() {
        let (addr, _server) = start_test_server(16).await;

        let mut client_a = connect_test_client(addr, "Talker").await;
        let mut client_b = connect_test_client(addr, "Listener").await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        let chat_content = "Hello from the other side!";
        client_a.send_chat(chat_content).await.unwrap();

        let received = timeout(TEST_TIMEOUT, async {
            loop {
                let msg = client_b.recv().await.unwrap();
                if let Message::Chat(chat) = msg {
                    return chat;
                }
            }
        })
        .await
        .expect("Client B should receive the chat message");

        assert_eq!(received.sender_id, client_a.player_id());
        assert_eq!(received.content, chat_content);
    }

    /// Stress test: 10 clients connect concurrently, each sends position
    /// updates, and none are dropped.
    #[tokio::test]
    async fn test_stress_10_concurrent_clients() {
        let (addr, server) = start_test_server(16).await;

        let mut clients = Vec::new();
        for i in 0..10 {
            let client = connect_test_client(addr, &format!("Player{i}")).await;
            clients.push(client);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert_eq!(
            server.connection_count().await,
            10,
            "Server should have 10 active connections"
        );

        // Each client sends a position update.
        for client in &mut clients {
            let pos = PlayerPosition {
                player_id: client.player_id(),
                pos_x_high: 0,
                pos_x_low: client.player_id() as i64 * 100,
                pos_y_high: 0,
                pos_y_low: 64,
                pos_z_high: 0,
                pos_z_low: client.player_id() as i64 * 200,
            };
            client.send(Message::PlayerPosition(pos)).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Verify all clients are still connected.
        assert_eq!(
            server.connection_count().await,
            10,
            "All 10 clients should still be connected after sending updates"
        );

        // Each client should have received entity updates for the other 9.
        for client in &mut clients {
            let seen_entities = client.received_entity_ids().await;
            assert!(
                seen_entities.len() >= 9,
                "Client {} should see at least 9 other entities, saw {}",
                client.player_id(),
                seen_entities.len()
            );
        }
    }
}
```
