# Player Join/Leave

## Problem

Players connecting to and disconnecting from a multiplayer server is a fundamental operation that must be handled gracefully. A joining player needs to receive the current world state to begin playing. A leaving player's entity and resources must be cleaned up, their state persisted, and other players notified. Unexpected disconnects (network failure, client crash) must be detected and handled identically to intentional leaves to prevent ghost entities and leaked resources.

## Solution

### Join Flow

When a client establishes a TCP connection to the server, the following sequence executes:

```
Client                          Server
  |                               |
  |--- ConnectionRequest -------->|
  |                               | Authenticate
  |<-- AuthResult(ok/fail) -------|
  |                               | Spawn player entity
  |                               | Compute initial interest area
  |<-- InitialWorldState ---------|  (nearby chunks + entities)
  |                               | Notify other clients
  |<-- JoinConfirmed -------------|
  |                               |
  |=== Normal gameplay begins ====|
```

#### 1. Authentication

The server validates the player's identity. For the initial implementation, this is a simple token-based check. The authentication system is extensible for future integration with external auth providers.

```rust
#[derive(Serialize, Deserialize)]
pub struct ConnectionRequest {
    pub player_name: String,
    pub auth_token: String,
    pub protocol_version: u32,
}

#[derive(Serialize, Deserialize)]
pub enum AuthResult {
    Accepted { client_id: ClientId, network_id: NetworkId },
    Rejected { reason: String },
}
```

#### 2. Player Entity Spawn

Upon successful authentication, the server spawns a player entity in the authoritative world:

```rust
pub fn spawn_player(
    world: &mut AuthoritativeWorld,
    client_id: ClientId,
    saved_state: Option<PlayerSaveData>,
) -> (Entity, NetworkId) {
    let network_id = world.allocate_network_id();

    let (position, inventory, health) = match saved_state {
        Some(save) => (save.position, save.inventory, save.health),
        None => (default_spawn_position(), Inventory::default(), Health::full()),
    };

    let entity = world.spawn((
        network_id,
        position,
        Rotation::default(),
        PlayerMarker,
        ClientOwner(client_id),
        inventory,
        health,
        InterestArea::default(),
    ));

    (entity, network_id)
}
```

#### 3. Initial World State

The server computes the player's initial interest area and sends:

- All voxel chunks within the interest radius (using the chunk streaming pipeline from Story 06, but front-loaded).
- All entity spawn messages for entities within the interest area (Story 02).
- Current server tick and world time.

```rust
#[derive(Serialize, Deserialize)]
pub struct InitialWorldState {
    pub your_network_id: NetworkId,
    pub server_tick: u64,
    pub world_time: f64,
    pub nearby_chunks: Vec<ChunkDataMessage>,
    pub nearby_entities: Vec<SpawnEntity>,
}
```

#### 4. Notify Others

All existing clients within range of the new player receive a `SpawnEntity` message for the new player entity via the normal replication system.

### Leave Flow

A player can leave voluntarily or involuntarily:

```rust
#[derive(Serialize, Deserialize)]
pub struct DisconnectRequest {
    pub reason: DisconnectReason,
}

#[derive(Serialize, Deserialize)]
pub enum DisconnectReason {
    Voluntary,   // Player quit
    Kicked,      // Server kicked
    Timeout,     // Connection lost
}
```

#### 1. Detect Disconnection

- **Voluntary**: Client sends `DisconnectRequest`.
- **Timeout**: Server detects no messages received within the timeout window (default: 30 seconds). A heartbeat message is expected every 5 seconds.

```rust
pub struct ConnectionState {
    pub client_id: ClientId,
    pub last_heartbeat: Instant,
    pub timeout_duration: Duration, // default: 30s
}

impl ConnectionState {
    pub fn is_timed_out(&self) -> bool {
        self.last_heartbeat.elapsed() > self.timeout_duration
    }
}
```

#### 2. Save Player State

Before despawning, the server persists the player's current state:

```rust
#[derive(Serialize, Deserialize)]
pub struct PlayerSaveData {
    pub player_name: String,
    pub position: Coord128,
    pub inventory: Inventory,
    pub health: Health,
    pub last_seen: u64, // server tick
}
```

#### 3. Despawn and Notify

The server despawns the player entity. The replication system (Story 02) automatically sends `DespawnEntity` to all clients who had this player in their interest area.

#### 4. Cleanup

- Remove client from the connection list.
- Remove client from all interest tracking (Story 03).
- Free the TCP connection resources.
- Remove per-client state (shadow state, chunk send queue, bandwidth tracker).

### Rejoin

When a player reconnects, the server loads their `PlayerSaveData` and spawns them at their last saved position with their previous inventory and health. This provides seamless session continuity.

## Outcome

- `nebula_multiplayer::session` module containing `ConnectionRequest`, `AuthResult`, `InitialWorldState`, `DisconnectRequest`, `DisconnectReason`, `ConnectionState`, `PlayerSaveData`, and `spawn_player`.
- Complete join flow: authenticate, spawn, send initial state, notify others.
- Complete leave flow: detect disconnect, save state, despawn, notify, cleanup.
- Timeout detection for unexpected disconnects.
- Player state persistence for rejoin continuity.

## Demo Integration

**Demo crate:** `nebula-demo`

When a new player joins, all existing players see a capsule appear. When a player disconnects, the capsule disappears with a brief fade-out.

## Crates & Dependencies

| Crate       | Version | Purpose                                         |
| ----------- | ------- | ----------------------------------------------- |
| `tokio`     | 1.49    | Async TCP, timers for heartbeat/timeout          |
| `serde`     | 1.0     | Serialization of session messages and save data  |
| `postcard`  | 1.1     | Binary wire format for join/leave messages       |
| `bevy_ecs`  | 0.18    | ECS entity spawn/despawn and component access    |

## Unit Tests

### `test_join_spawns_entity_visible_to_others`
Client A is connected. Client B joins. Assert that Client A receives a `SpawnEntity` message for Client B's player entity with correct `NetworkId`, position, and components.

### `test_leave_despawns_entity`
Client A and Client B are connected and within each other's interest area. Client B disconnects. Assert that Client A receives a `DespawnEntity` message for Client B's `NetworkId` and the entity no longer exists in the server world.

### `test_initial_state_includes_nearby_data`
Client joins at a position surrounded by loaded chunks and entities. Assert the `InitialWorldState` contains chunk data for all chunks within the interest radius and `SpawnEntity` messages for all entities within range.

### `test_other_players_are_notified`
Three clients are connected within mutual interest areas. Client D joins nearby. Assert that all three existing clients receive a `SpawnEntity` message for Client D.

### `test_state_persists_across_rejoin`
Client A joins, moves to position P, modifies inventory. Client A disconnects. Client A reconnects. Assert the spawned player entity has position P and the modified inventory, not the defaults.
