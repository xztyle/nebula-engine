# Voxel Type Registry

## Problem

Every voxel in the engine must have a well-defined type that describes its visual and physical properties — whether it is solid, transparent, what material it uses, and how much light it emits. Without a centralized registry, voxel type information would be scattered across systems, leading to inconsistent lookups, duplicate definitions, and no single source of truth. The engine needs a compact numeric identifier for each voxel type (to keep chunk storage small) while still allowing systems to query rich metadata by that identifier. Additionally, Air must always be ID 0 so that zero-initialized chunk memory is immediately valid as empty space — a critical invariant for both correctness and performance.

## Solution

Introduce a `VoxelTypeRegistry` in the `nebula-voxel` crate that maps a `VoxelTypeId` (a `u16` newtype) to a `VoxelTypeDef` struct. The registry is populated during engine startup and becomes immutable once the game begins simulation.

### Data Structures

```rust
/// Compact identifier stored inside every voxel cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VoxelTypeId(pub u16);

/// Full descriptor for a voxel type.
#[derive(Clone, Debug)]
pub struct VoxelTypeDef {
    /// Human-readable name (e.g., "stone", "grass", "water").
    pub name: String,
    /// Whether entities collide with this voxel.
    pub solid: bool,
    /// Transparency mode: Opaque, SemiTransparent, or FullyTransparent.
    pub transparency: Transparency,
    /// Index into the material palette (albedo, roughness, etc.).
    pub material_index: u16,
    /// Light emission level (0 = none, 15 = max, following Minecraft convention).
    pub light_emission: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transparency {
    Opaque,
    SemiTransparent,
    FullyTransparent,
}
```

### Registry Implementation

```rust
pub struct VoxelTypeRegistry {
    /// Dense array: index == VoxelTypeId.0
    types: Vec<VoxelTypeDef>,
    /// Reverse lookup for name -> ID.
    name_to_id: HashMap<String, VoxelTypeId>,
}
```

The registry is constructed with `VoxelTypeRegistry::new()`, which automatically registers Air as ID 0 with `solid: false`, `transparency: FullyTransparent`, `material_index: 0`, and `light_emission: 0`.

### API

- **`register(def: VoxelTypeDef) -> Result<VoxelTypeId, RegistryError>`** — Appends a new type and returns the assigned ID. Fails with `RegistryError::DuplicateName` if the name already exists, or `RegistryError::RegistryFull` if all 65535 non-air slots are consumed. IDs are assigned sequentially starting from 1.
- **`get(id: VoxelTypeId) -> &VoxelTypeDef`** — Returns the definition for a given ID. Panics if the ID is out of range (this is a programming error, not a runtime condition, since IDs come from the registry itself).
- **`lookup_by_name(name: &str) -> Option<VoxelTypeId>`** — Returns the ID for a named type, or `None` if no type with that name has been registered.
- **`len() -> usize`** — Returns the total number of registered types, including Air.

The registry is designed to be built once and shared via `Arc<VoxelTypeRegistry>` as a Bevy ECS resource. No mutation methods are exposed after the initial registration phase; the builder pattern or a dedicated `RegistryBuilder` can enforce this at the type level if desired.

### Maximum Capacity

With `VoxelTypeId` as `u16`, the theoretical maximum is 65536 types (IDs 0 through 65535). Since ID 0 is reserved for Air, 65535 user-defined types are available. In practice, a game will register hundreds to low thousands of types — the u16 ceiling is generous but keeps per-voxel storage at exactly 2 bytes before palette compression.

## Outcome

A `VoxelTypeRegistry` struct in `nebula-voxel` that compiles and passes all unit tests. Air is guaranteed to be ID 0. Other systems (meshing, lighting, physics) can query voxel properties by ID in O(1) time via direct array indexing. Name-based lookup supports editor tooling and scripting.

## Demo Integration

**Demo crate:** `nebula-demo`

The demo registers a small palette: Air (0), Stone (1), Dirt (2), Grass (3). The title shows `Registry: 4 types`. Each type has a debug color.

## Crates & Dependencies

- **`hashbrown`** `0.15` — Fast hash map for name-to-ID reverse lookup (or use `std::collections::HashMap` with no extra dependency)
- **`thiserror`** `2.0` — Ergonomic error type derivation for `RegistryError`
- **`serde`** `1.0` with `derive` feature — Serialize/deserialize `VoxelTypeDef` for asset loading and network sync

## Unit Tests

- **`test_air_is_id_zero`** — Create a new `VoxelTypeRegistry` and assert that `registry.get(VoxelTypeId(0)).name == "air"`, `solid == false`, and `transparency == FullyTransparent`.
- **`test_register_returns_sequential_ids`** — Register three types ("stone", "dirt", "grass") and assert the returned IDs are `VoxelTypeId(1)`, `VoxelTypeId(2)`, `VoxelTypeId(3)` respectively.
- **`test_lookup_by_name`** — Register a type named "obsidian", then assert `registry.lookup_by_name("obsidian") == Some(VoxelTypeId(_))` and `registry.lookup_by_name("nonexistent") == None`.
- **`test_get_returns_correct_def`** — Register a type with `solid: true`, `material_index: 42`, `light_emission: 12`, then retrieve it by ID and assert all fields match the original definition.
- **`test_duplicate_name_rejected`** — Register a type named "stone", then attempt to register another type also named "stone" and assert the result is `Err(RegistryError::DuplicateName)`.
