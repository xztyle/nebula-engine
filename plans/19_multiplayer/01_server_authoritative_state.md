# Server-Authoritative State

## Problem

In a multiplayer game engine with 128-bit coordinates and cubesphere-voxel planets, allowing clients to directly modify world state opens the door to cheating, desynchronization, and inconsistency. Without a single source of truth, two clients observing the same region of space can see divergent realities. The server must own the canonical game state so that every connected player experiences a consistent, validated world.

## Solution

The server runs the full ECS simulation as the sole authority over game state. Clients never modify world state directly. Instead, they send **input intents** (movement vectors, action requests, voxel edit intents) over the pure TCP connection to the server. The server validates every intent against its authoritative state before applying it.

### Architecture

- **`AuthoritativeWorld`** — a wrapper around `bevy_ecs::World` that lives exclusively on the server. All game logic systems operate on this world instance.
- **`ClientIntent`** — a serde-serializable enum representing every possible client action:
  ```rust
  #[derive(Serialize, Deserialize)]
  enum ClientIntent {
      MoveDirection { tick: u64, direction: Vec3_128 },
      PlaceVoxel { chunk_id: ChunkId, local_pos: UVec3, material: VoxelMaterial },
      RemoveVoxel { chunk_id: ChunkId, local_pos: UVec3 },
      UseItem { slot: u8, target: Option<NetworkId> },
      Interact { target: NetworkId },
  }
  ```
- **`IntentValidator`** — a system that inspects each incoming intent against the current world state. Checks include range validation (is the target within reach?), permission checks (does the player own the item?), and physics feasibility (is the destination reachable?).
- **`ServerTickSchedule`** — the server advances the simulation at a fixed 60 Hz tick rate. Each tick: (1) read all queued client intents, (2) validate and apply valid intents, (3) run physics and game logic systems, (4) diff the resulting state against the previous tick, (5) broadcast authoritative state deltas to clients.

### Authoritative Systems

The server is the sole authority over:

| System               | Description                                                  |
| -------------------- | ------------------------------------------------------------ |
| Entity Positions     | All entity transforms are computed server-side using 128-bit coordinates. Clients receive position updates. |
| Voxel Modifications  | Block place/remove operations are validated and applied on the server before broadcast. |
| Inventory            | Item ownership, stack counts, and transfers are server-controlled. |
| Health & Combat      | Damage calculations, death, respawn are fully server-side.   |

### Anti-Cheat

Because clients only submit intents and never state, cheating is structurally limited. The server can detect and reject:

- Movement intents that would exceed maximum velocity.
- Voxel edits targeting chunks outside the player's reach radius.
- Item usage for items not present in the player's inventory.
- Action frequency exceeding the tick rate (intent flooding).

Invalid intents are silently dropped and optionally logged for review.

### Serialization

All network messages use `postcard` for compact binary serialization over TCP. The `ClientIntent` and `ServerStateUpdate` types derive `Serialize` and `Deserialize` via `serde`.

```rust
use postcard::to_allocvec;

let bytes = to_allocvec(&intent).expect("serialization failed");
tcp_stream.write_all(&bytes).await?;
```

## Outcome

- `nebula_multiplayer::authority` module containing `AuthoritativeWorld`, `ClientIntent`, `IntentValidator`, and `ServerTickSchedule`.
- Server binary that runs the full ECS simulation at 60 Hz, accepts client TCP connections, validates intents, and broadcasts authoritative state.
- Integration with the networking layer from Epic 18 for TCP message framing.

## Demo Integration

**Demo crate:** `nebula-demo`

The server owns the canonical game state. Client inputs are sent as intents; the server validates and applies them. No client-side state modification is authoritative.

## Crates & Dependencies

| Crate       | Version | Purpose                              |
| ----------- | ------- | ------------------------------------ |
| `tokio`     | 1.49    | Async runtime for TCP server         |
| `serde`     | 1.0     | Serialization derive for intents     |
| `postcard`  | 1.1     | Compact binary wire format           |
| `bevy_ecs`  | 0.18    | ECS world and system scheduling      |

## Unit Tests

### `test_client_input_validated_by_server`
Submit a valid `MoveDirection` intent. Assert the server applies it and the entity position changes according to the direction vector within the next tick.

### `test_server_state_overrides_client_prediction`
Client predicts position `P_client`. Server computes a different position `P_server` (e.g., due to collision). Assert that after reconciliation the client's visible state matches `P_server`.

### `test_invalid_action_rejected`
Submit a `PlaceVoxel` intent targeting a chunk 10,000 m from the player. Assert the server drops the intent and the voxel remains unchanged.

### `test_server_tick_produces_consistent_state`
Run the server for 100 ticks with deterministic inputs. Assert the resulting entity positions and voxel state match the expected output exactly.

### `test_two_clients_see_same_world_state`
Connect two clients to the same server. Both observe the same entity. Assert that after the server broadcasts, both clients receive identical position and component data for that entity.
