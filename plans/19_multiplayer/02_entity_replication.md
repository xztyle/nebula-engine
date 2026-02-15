# Entity Replication

## Problem

The server runs the full ECS simulation with hundreds of component types, but clients only need a subset of those components to render and interact with the world. Sending every component for every entity every tick would be wasteful and leak server-internal state (AI decision trees, server-side cooldown timers, cheat-detection metadata). The engine needs a selective, bandwidth-efficient replication system that delivers only relevant component data to clients.

## Solution

### Network Identity

Every entity that participates in replication is assigned a `NetworkId` — a unique `u64` identifier that is stable across the network. The server allocates `NetworkId` values from a monotonically increasing counter. Clients reference entities exclusively by `NetworkId`, never by their local ECS `Entity` handle.

```rust
#[derive(Component, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NetworkId(pub u64);
```

### Replication Sets

A `ReplicationSet` defines which components should be replicated for a given entity archetype. Components are categorized:

| Category        | Examples                                | Replicated |
| --------------- | --------------------------------------- | ---------- |
| **Shared**      | Position, Rotation, Scale, MeshHandle   | Yes        |
| **Visual**      | AnimationState, ParticleEmitter         | Yes        |
| **Server-Only** | AiBrain, ServerCooldown, CheatFlags     | No         |
| **Client-Only** | InputBuffer, PredictionState            | No         |

```rust
pub struct ReplicationSet {
    pub components: Vec<ComponentDescriptor>,
}

pub struct ComponentDescriptor {
    pub type_id: TypeId,
    pub serializer: fn(&World, Entity) -> Option<Vec<u8>>,
    pub deserializer: fn(&mut World, Entity, &[u8]),
}
```

Registration at startup:

```rust
replication.register::<Position128>();
replication.register::<Rotation>();
replication.register::<MeshHandle>();
replication.register::<AnimationState>();
```

### Delta Compression

Each tick, the replication system compares the current value of each replicated component against its value at the last acknowledged tick for each client. Only changed components are included in the update message.

```rust
#[derive(Serialize, Deserialize)]
pub struct EntityUpdate {
    pub network_id: NetworkId,
    pub tick: u64,
    pub changed_components: Vec<(ComponentTypeTag, Vec<u8>)>,
}
```

The server maintains a **per-client shadow state** — a copy of what the client last acknowledged. When a component changes relative to the shadow, it is included in the next update. When the client acknowledges a tick, the shadow advances.

### Spawn and Despawn

When a new entity enters a client's interest area (see Story 03), the server sends a `SpawnEntity` message containing the full component set:

```rust
#[derive(Serialize, Deserialize)]
pub struct SpawnEntity {
    pub network_id: NetworkId,
    pub components: Vec<(ComponentTypeTag, Vec<u8>)>,
}
```

When an entity is destroyed or leaves the interest area, a `DespawnEntity` message is sent:

```rust
#[derive(Serialize, Deserialize)]
pub struct DespawnEntity {
    pub network_id: NetworkId,
}
```

### Serialization

All replication messages are serialized with `postcard` via `serde`. Component data is serialized individually, allowing heterogeneous component types in a single update message.

## Outcome

- `nebula_multiplayer::replication` module containing `NetworkId`, `ReplicationSet`, `ComponentDescriptor`, `EntityUpdate`, `SpawnEntity`, `DespawnEntity`.
- `ReplicationServerSystem` that runs each tick to diff component state and produce update messages per client.
- `ReplicationClientSystem` that receives updates and applies them to the local ECS world.
- Per-client shadow state tracking for efficient delta computation.

## Demo Integration

**Demo crate:** `nebula-demo`

The other player's capsule appears in each client's world. Entity positions are replicated from server to all interested clients in real time.

## Crates & Dependencies

| Crate       | Version | Purpose                                    |
| ----------- | ------- | ------------------------------------------ |
| `tokio`     | 1.49    | Async TCP for sending replication messages  |
| `serde`     | 1.0     | Derive Serialize/Deserialize on components  |
| `postcard`  | 1.1     | Binary serialization of update messages     |
| `bevy_ecs`  | 0.18    | ECS World, Entity, Component access         |

## Unit Tests

### `test_entity_spawned_on_server_appears_on_client`
Spawn an entity with `NetworkId`, `Position128`, and `MeshHandle` on the server. Run the replication system. Assert the client receives a `SpawnEntity` message containing all three components with correct values.

### `test_component_change_replicates`
Modify the `Position128` of a replicated entity on the server. Run the replication system. Assert the client receives an `EntityUpdate` containing only the `Position128` component with the new value.

### `test_unchanged_components_not_sent`
Run two consecutive ticks without modifying any components. Assert the second tick produces no `EntityUpdate` messages (or an empty changed_components list) for unchanged entities.

### `test_network_id_is_consistent`
Spawn an entity on the server with `NetworkId(42)`. Replicate to the client. Assert the client's local entity has a `NetworkId` component with value `42`, and that subsequent updates reference the same `NetworkId`.

### `test_despawn_replicates`
Despawn a replicated entity on the server. Run the replication system. Assert the client receives a `DespawnEntity` message with the correct `NetworkId`, and the client removes the entity from its local world.
