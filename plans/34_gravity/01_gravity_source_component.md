# Gravity Source Component

## Problem

Nebula Engine supports cubesphere-voxel planets, moons, and asteroids — each of which exerts gravitational pull on nearby entities. The physics simulation (rapier3d with world gravity set to zero) relies on per-entity gravity forces, which means the engine must know which entities are gravity sources, what their mass and radius are, and what the pre-computed surface gravity is for each. Without a dedicated ECS component and registry, every system that needs gravity information would have to redundantly discover and compute it. The engine needs a single source of truth: a `GravitySource` component that marks an entity as a gravitational attractor and stores the parameters required for gravity field computation. A registry (spatial index) of all gravity sources must exist so that any system can efficiently query "which gravity sources are near this position?" without scanning every entity in the world.

## Solution

### GravitySource Component

Define a `GravitySource` component in the `nebula-gravity` crate that stores the physical parameters of a gravitational body:

```rust
use bevy_ecs::component::Component;

#[derive(Component, Debug, Clone)]
pub struct GravitySource {
    /// Mass of the body in engine mass units (kg-equivalent).
    /// Used for precise inverse-square computation at distance.
    pub mass: f64,

    /// Radius of the body's surface, stored in i128 world units.
    /// Matches the engine's 128-bit coordinate system for exact
    /// surface boundary detection.
    pub radius: i128,

    /// Pre-computed surface gravity in m/s².
    /// For a planet with known surface conditions, this avoids
    /// computing G*M/r² every tick for on-surface entities.
    /// The gravity field computation uses this as the reference:
    /// g(r) = surface_gravity * (radius / distance)²
    pub surface_gravity: f32,

    /// Maximum influence radius in i128 world units.
    /// Beyond this distance, the source contributes zero gravity.
    /// Prevents distant, irrelevant sources from entering calculations.
    pub influence_radius: i128,
}
```

Surface gravity is pre-computed at registration time. For a planet with known mass `M` and radius `R`, `surface_gravity = (G * M) / R²`. This value is stored directly so that on-surface entities (the overwhelmingly common case) never need to evaluate the inverse-square law — they simply read `surface_gravity` and use the direction toward the planet center. For entities at altitude, the field computation uses `surface_gravity * (radius / distance)²`, which is algebraically equivalent to the full Newtonian formula but avoids recomputing the constant numerator.

### GravitySourceRegistry Resource

A resource that maintains a spatial index of all active gravity sources for fast nearest-source queries:

```rust
use bevy_ecs::system::Resource;
use bevy_ecs::entity::Entity;

#[derive(Resource, Default)]
pub struct GravitySourceRegistry {
    /// All registered gravity sources with their world positions.
    /// Sorted by influence radius (largest first) for early-exit queries.
    sources: Vec<GravitySourceEntry>,
}

pub struct GravitySourceEntry {
    pub entity: Entity,
    pub position: WorldPos,
    pub source: GravitySource,
}

impl GravitySourceRegistry {
    /// Register or update a gravity source.
    pub fn register(&mut self, entity: Entity, position: WorldPos, source: GravitySource) { .. }

    /// Remove a gravity source (entity despawned or no longer a source).
    pub fn unregister(&mut self, entity: Entity) { .. }

    /// Query all sources within influence range of the given position.
    pub fn sources_affecting(&self, position: &WorldPos) -> Vec<&GravitySourceEntry> { .. }

    /// Find the single nearest (strongest) gravity source.
    pub fn nearest_source(&self, position: &WorldPos) -> Option<&GravitySourceEntry> { .. }

    /// Total number of registered sources.
    pub fn count(&self) -> usize { .. }
}
```

### Registry Sync System

A system that runs each tick to synchronize the registry with the ECS world. When a `GravitySource` component is added, modified, or removed, the registry updates accordingly:

```rust
fn sync_gravity_sources(
    sources: Query<(Entity, &WorldPos, &GravitySource), Changed<GravitySource>>,
    removed: RemovedComponents<GravitySource>,
    mut registry: ResMut<GravitySourceRegistry>,
) {
    for (entity, pos, source) in sources.iter() {
        registry.register(entity, *pos, source.clone());
    }
    for entity in removed.read() {
        registry.unregister(entity);
    }
}
```

### Surface Gravity Pre-computation

A helper function computes `surface_gravity` from mass and radius, so callers constructing planets do not need to know the gravitational constant:

```rust
/// Gravitational constant in engine units (m³ kg⁻¹ s⁻²).
pub const G: f64 = 6.674_30e-11;

/// Compute surface gravity from mass and radius.
/// Returns g = G * mass / radius², where radius is converted from i128 world units to f64 meters.
pub fn compute_surface_gravity(mass: f64, radius: i128) -> f32 {
    let r = radius as f64;
    ((G * mass) / (r * r)) as f32
}
```

## Outcome

The `nebula-gravity` crate exports the `GravitySource` component, `GravitySourceRegistry` resource, the `sync_gravity_sources` system, and the `compute_surface_gravity` helper. Any entity with a `GravitySource` component is automatically tracked in the registry. Other gravity systems (field computation, blending, orientation alignment) query the registry rather than scanning all entities. `cargo test -p nebula-gravity` passes all gravity source component and registry tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Two planets are placed in the demo, each with a `GravitySource` component defining different mass and radius. Their gravity strengths differ visibly.

## Crates & Dependencies

- `bevy_ecs = "0.18"` — ECS framework for `Component` derive, `Resource`, `Query`, `Changed` filter, `RemovedComponents`, and system signatures
- `glam = "0.32"` — Vector math types used internally for distance calculations
- `nebula-math` (internal) — `WorldPos` type with i128 coordinates for precise spatial queries
- `nebula-coords` (internal) — Coordinate conversion utilities for i128-to-f64 distance computation

## Unit Tests

- **`test_gravity_source_stores_values`** — Construct a `GravitySource { mass: 5.972e24, radius: 6_371_000, surface_gravity: 9.81, influence_radius: 100_000_000 }`. Assert each field retains its value exactly: `mass == 5.972e24`, `radius == 6_371_000_i128`, `surface_gravity == 9.81_f32`, `influence_radius == 100_000_000_i128`. Verifies the component is a plain data struct with no transformation on construction.

- **`test_registry_tracks_sources`** — Create a `GravitySourceRegistry`. Register three gravity sources (a planet, a moon, an asteroid) with distinct entity IDs. Assert `registry.count() == 3`. Unregister the moon. Assert `registry.count() == 2`. Verifies add and remove operations.

- **`test_surface_gravity_precomputed`** — Call `compute_surface_gravity(5.972e24, 6_371_000)`. Assert the result is approximately `9.81` within `0.02` tolerance. Verifies the pre-computation formula `G * M / R²` produces the expected Earth surface gravity.

- **`test_multiple_sources_registered`** — Register five gravity sources at different world positions. Call `sources_affecting(&position)` for a position within influence range of exactly two sources. Assert the returned list has length 2. Assert both expected entities are present in the list and no others.

- **`test_query_nearest_gravity_source`** — Register two sources: planet A at `WorldPos(0, 0, 0)` with `radius = 6_371_000` and planet B at `WorldPos(50_000_000, 0, 0)` with `radius = 3_000_000`. Query `nearest_source` from `WorldPos(10_000_000, 0, 0)` (closer to A). Assert the returned entity is planet A. Query from `WorldPos(45_000_000, 0, 0)` (closer to B). Assert the returned entity is planet B.
