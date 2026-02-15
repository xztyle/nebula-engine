# Spatial Interest Management

## Problem

A Nebula Engine server may simulate thousands of entities across a cubesphere-voxel planet with 128-bit coordinate space. Sending every entity update to every client is prohibitively expensive in bandwidth and wasteful — a player standing on one face of the planet does not need updates about entities on the opposite face 12,000 km away. The engine needs a system that determines which entities are relevant to each client and only replicates those.

## Solution

### Interest Area

Each connected client has an **interest area** defined as a sphere centered on the client's player entity position. The default radius is 500 meters, configurable per-client or per-server.

```rust
#[derive(Component)]
pub struct InterestArea {
    pub radius: f64,
}

impl Default for InterestArea {
    fn default() -> Self {
        Self { radius: 500.0 }
    }
}
```

### Spatial Hash Integration

The system leverages the spatial hash from Epic 03 (coordinate systems) to efficiently query entities within a given radius. The spatial hash partitions 128-bit coordinate space into cells. For each client, the system queries all cells overlapping the interest sphere and collects entities within those cells.

```rust
pub fn entities_in_interest_area(
    spatial_hash: &SpatialHash128,
    center: &Coord128,
    radius: f64,
) -> Vec<Entity> {
    spatial_hash.query_sphere(center, radius)
}
```

### State Transitions

Entities transition between three states relative to each client:

| Transition         | Action                                                         |
| ------------------ | -------------------------------------------------------------- |
| **Enter interest** | Send `SpawnEntity` with full component state                   |
| **Stay inside**    | Send `EntityUpdate` with delta-compressed changes (Story 02)   |
| **Leave interest** | Send `DespawnEntity` to remove from client world               |

The server maintains a **per-client interest set** — the set of `NetworkId` values currently within the client's interest area. Each tick:

1. Query the spatial hash for all entities in the client's interest sphere.
2. Compare against the previous tick's interest set.
3. New entries: send `SpawnEntity`.
4. Removed entries: send `DespawnEntity`.
5. Continuing entries: handled by the replication delta system (Story 02).

```rust
pub struct ClientInterestSet {
    pub client_id: ClientId,
    pub current: HashSet<NetworkId>,
    pub previous: HashSet<NetworkId>,
}

impl ClientInterestSet {
    pub fn compute_transitions(&self) -> InterestTransitions {
        InterestTransitions {
            entered: self.current.difference(&self.previous).copied().collect(),
            exited: self.previous.difference(&self.current).copied().collect(),
        }
    }
}
```

### 128-Bit Distance Calculation

Distance checks use the engine's 128-bit coordinate system. Since interest radii are relatively small (hundreds of meters), the distance calculation can safely downcast to `f64` after computing the offset between two `Coord128` values, avoiding full 128-bit floating-point arithmetic.

```rust
pub fn within_interest(a: &Coord128, b: &Coord128, radius: f64) -> bool {
    let delta = a.offset_to(b);
    let dist_sq = delta.x_f64().powi(2) + delta.y_f64().powi(2) + delta.z_f64().powi(2);
    dist_sq <= radius * radius
}
```

### Cubesphere Awareness

On a cubesphere planet, "nearby" entities might be on adjacent cube faces. The spatial hash handles face boundaries transparently (see Epic 03/05), so interest queries near face edges correctly include entities on neighboring faces.

## Outcome

- `nebula_multiplayer::interest` module containing `InterestArea`, `ClientInterestSet`, `InterestTransitions`, and the `SpatialInterestSystem`.
- Per-client interest tracking that integrates with the replication system from Story 02.
- Efficient spatial queries via the Epic 03 spatial hash, operating in 128-bit coordinate space.
- Configurable interest radius per client.

## Demo Integration

**Demo crate:** `nebula-demo`

Only entities near the player are replicated. A distant player on the other side of the planet does not consume bandwidth. The interest radius is configurable.

## Crates & Dependencies

| Crate       | Version | Purpose                                        |
| ----------- | ------- | ---------------------------------------------- |
| `bevy_ecs`  | 0.18    | ECS queries for entity positions and components |
| `serde`     | 1.0     | Serialization of interest configuration         |
| `postcard`  | 1.1     | Wire format for spawn/despawn messages          |
| `tokio`     | 1.49    | Async context for sending interest updates      |

## Unit Tests

### `test_entity_inside_area_is_replicated`
Place an entity 100 m from the client (radius 500 m). Run the interest system. Assert the entity's `NetworkId` is in the client's interest set and a `SpawnEntity` message is generated.

### `test_entity_outside_area_is_not_replicated`
Place an entity 1000 m from the client (radius 500 m). Run the interest system. Assert the entity's `NetworkId` is not in the client's interest set and no messages are generated for it.

### `test_entity_entering_area_triggers_full_state_send`
Start with an entity at 600 m (outside). Move it to 400 m (inside). Run the interest system. Assert the entity transitions from absent to present in the interest set, and a `SpawnEntity` message with full component data is generated.

### `test_entity_leaving_area_triggers_despawn`
Start with an entity at 400 m (inside, already replicated). Move it to 600 m (outside). Run the interest system. Assert the entity transitions from present to absent, and a `DespawnEntity` message is generated.

### `test_interest_radius_is_configurable`
Set the client's `InterestArea` radius to 200 m. Place an entity at 300 m. Run the interest system. Assert the entity is not in the interest set. Change the radius to 400 m. Re-run. Assert the entity is now in the interest set.
