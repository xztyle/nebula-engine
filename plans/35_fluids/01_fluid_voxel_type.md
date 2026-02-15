# Fluid Voxel Type

## Problem

The voxel engine currently treats every block as either solid or air. There is no representation for materials that flow, fill partial volumes, and interact with entities through buoyancy and drag rather than collision. Water, lava, oil, and other liquids are fundamentally different from solid blocks: they are transparent, non-solid, occupy partial volumes (a block can be 3/8 full of water), and carry per-type physical properties like viscosity and flow speed. Without a dedicated fluid voxel category, the engine cannot simulate oceans, rivers, lava flows, or any liquid body on a cubesphere planet. The fluid type must integrate cleanly with the existing `VoxelTypeRegistry` (Epic 06, story 01) while adding fluid-specific metadata that solid voxels do not need.

## Solution

### FluidTypeId and FluidTypeDef

Define a separate fluid type registry that sits alongside the voxel type registry. Each fluid type has an identifier and a descriptor with physical and visual properties:

```rust
/// Identifier for a specific fluid (Water, Lava, Oil, etc.).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FluidTypeId(pub u8);

/// Describes the physical and visual properties of a fluid type.
#[derive(Clone, Debug)]
pub struct FluidTypeDef {
    /// Human-readable name (e.g., "water", "lava", "oil").
    pub name: String,
    /// Dynamic viscosity. Higher values mean slower flow.
    /// Water ~1.0, Oil ~5.0, Lava ~50.0 (arbitrary game units).
    pub viscosity: f32,
    /// Base color and alpha for rendering (RGBA, premultiplied alpha).
    pub color: [f32; 4],
    /// Flow speed multiplier. Combined with viscosity to determine
    /// how many ticks between cellular automaton updates.
    pub flow_speed: f32,
    /// Density in kg/m^3 (game-scale). Determines buoyancy.
    /// Water = 1000.0, Lava = 3100.0, Oil = 800.0.
    pub density: f32,
    /// Damage per second dealt to entities submerged in this fluid.
    /// Zero for water and oil, positive for lava.
    pub damage_per_second: f32,
    /// Whether this fluid emits light (lava glows).
    pub light_emission: u8,
}
```

### FluidVoxel

A fluid voxel is stored in the chunk data alongside solid voxels. The existing `VoxelTypeId` space already supports non-solid types (transparency, `solid: false`). A fluid voxel is a voxel whose type is registered with `is_fluid: true` in the `VoxelTypeDef`, and whose auxiliary data encodes the fluid level:

```rust
/// Fluid state packed into the auxiliary byte of a voxel cell.
/// Bits [0..3]: level (0 = empty, 7 = full block)
/// Bits [3..7]: reserved for future flags (e.g., flow direction hint)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FluidState {
    /// Which fluid occupies this cell.
    pub fluid_type: FluidTypeId,
    /// Fill level: 0 = empty (no fluid), 1..=7 = partial to full.
    pub level: u8,
}

impl FluidState {
    pub const EMPTY: u8 = 0;
    pub const FULL: u8 = 7;

    pub fn new(fluid_type: FluidTypeId, level: u8) -> Self {
        assert!(level <= Self::FULL, "Fluid level must be 0..=7");
        Self { fluid_type, level }
    }

    pub fn is_empty(&self) -> bool {
        self.level == Self::EMPTY
    }

    pub fn is_full(&self) -> bool {
        self.level == Self::FULL
    }
}
```

### Integration with VoxelTypeRegistry

Extend `VoxelTypeDef` with an optional fluid reference:

```rust
// In nebula-voxel VoxelTypeDef:
pub struct VoxelTypeDef {
    pub name: String,
    pub solid: bool,
    pub transparency: Transparency,
    pub material_index: u16,
    pub light_emission: u8,
    /// If this voxel type represents a fluid, the associated FluidTypeId.
    /// `None` for solid and air types.
    pub fluid_id: Option<FluidTypeId>,
}
```

When a voxel type is registered with `fluid_id: Some(id)`, the registry enforces that `solid == false` and `transparency` is `SemiTransparent`. This ensures fluids are always non-solid and transparent without requiring callers to remember these constraints.

### FluidTypeRegistry

```rust
pub struct FluidTypeRegistry {
    types: Vec<FluidTypeDef>,
    name_to_id: HashMap<String, FluidTypeId>,
}

impl FluidTypeRegistry {
    pub fn new() -> Self {
        Self {
            types: Vec::new(),
            name_to_id: HashMap::new(),
        }
    }

    pub fn register(&mut self, def: FluidTypeDef) -> Result<FluidTypeId, RegistryError> {
        if self.name_to_id.contains_key(&def.name) {
            return Err(RegistryError::DuplicateName);
        }
        if self.types.len() >= 256 {
            return Err(RegistryError::RegistryFull);
        }
        let id = FluidTypeId(self.types.len() as u8);
        self.name_to_id.insert(def.name.clone(), id);
        self.types.push(def);
        Ok(id)
    }

    pub fn get(&self, id: FluidTypeId) -> &FluidTypeDef {
        &self.types[id.0 as usize]
    }

    pub fn lookup_by_name(&self, name: &str) -> Option<FluidTypeId> {
        self.name_to_id.get(name).copied()
    }
}
```

The registry is shared as an `Arc<FluidTypeRegistry>` ECS resource, built during startup alongside the voxel type registry.

### Chunk Storage

Each chunk already stores a `VoxelTypeId` per cell. The fluid level is stored in a parallel `FluidState` array only for chunks that contain at least one fluid voxel (lazy allocation). This avoids bloating memory for the vast majority of chunks that are entirely solid or air:

```rust
pub struct ChunkFluidData {
    /// Parallel array to the chunk's voxel data. Index by the same
    /// local coordinate. Only allocated when a chunk contains fluid.
    states: Box<[FluidState; CHUNK_VOLUME]>,
}
```

`CHUNK_VOLUME` is `CHUNK_SIZE^3` (e.g., 32^3 = 32768). Each `FluidState` is 2 bytes (`FluidTypeId` u8 + level u8), so the total per-chunk cost is 64 KiB when allocated.

### Fluid Properties at a Glance

| Fluid | Viscosity | Flow Speed | Density | Damage/s | Light | Color (RGBA) |
|-------|-----------|------------|---------|----------|-------|-------------|
| Water | 1.0 | 1.0 | 1000.0 | 0.0 | 0 | (0.2, 0.4, 0.9, 0.6) |
| Lava | 50.0 | 0.1 | 3100.0 | 20.0 | 14 | (1.0, 0.3, 0.0, 0.95) |
| Oil | 5.0 | 0.5 | 800.0 | 0.0 | 0 | (0.15, 0.1, 0.05, 0.8) |

## Outcome

The `nebula-voxel` crate exports `FluidTypeId`, `FluidTypeDef`, `FluidState`, `FluidTypeRegistry`, and `ChunkFluidData`. Fluid types are registered at startup and referenced by a compact `FluidTypeId`. Each fluid voxel stores its type and a 3-bit fill level (0-7). The voxel type registry links fluid voxels to their `FluidTypeId` via `VoxelTypeDef::fluid_id`. Running `cargo test -p nebula-voxel` passes all fluid voxel type tests. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Water and lava are registered as fluid voxel types with a "level" property (1-8) representing fill amount. Placing a water source creates a full-level water voxel.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `hashbrown` | `0.15` | Fast hash map for fluid name-to-ID reverse lookup |
| `thiserror` | `2.0` | Ergonomic error types for `RegistryError` |
| `serde` | `1.0` | Serialize/deserialize `FluidTypeDef` for asset loading and network sync |
| `bevy_ecs` | `0.18` | Store `FluidTypeRegistry` as a shared ECS resource |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fluid_voxel_stores_type_and_level() {
        let state = FluidState::new(FluidTypeId(0), 5);
        assert_eq!(state.fluid_type, FluidTypeId(0));
        assert_eq!(state.level, 5);
    }

    #[test]
    fn test_level_range_is_0_to_7() {
        // Level 0 (empty) is valid
        let empty = FluidState::new(FluidTypeId(0), 0);
        assert!(empty.is_empty());

        // Level 7 (full) is valid
        let full = FluidState::new(FluidTypeId(0), 7);
        assert!(full.is_full());

        // All intermediate levels are valid
        for level in 0..=7 {
            let state = FluidState::new(FluidTypeId(0), level);
            assert_eq!(state.level, level);
        }
    }

    #[test]
    #[should_panic(expected = "Fluid level must be 0..=7")]
    fn test_level_above_7_panics() {
        FluidState::new(FluidTypeId(0), 8);
    }

    #[test]
    fn test_fluid_is_transparent() {
        // When registering a fluid voxel type, transparency must be SemiTransparent
        let mut voxel_registry = VoxelTypeRegistry::new();
        let mut fluid_registry = FluidTypeRegistry::new();

        let water_fluid_id = fluid_registry.register(FluidTypeDef {
            name: "water".to_string(),
            viscosity: 1.0,
            color: [0.2, 0.4, 0.9, 0.6],
            flow_speed: 1.0,
            density: 1000.0,
            damage_per_second: 0.0,
            light_emission: 0,
        }).unwrap();

        let water_voxel_id = voxel_registry.register(VoxelTypeDef {
            name: "water".to_string(),
            solid: false,
            transparency: Transparency::SemiTransparent,
            material_index: 0,
            light_emission: 0,
            fluid_id: Some(water_fluid_id),
        }).unwrap();

        let def = voxel_registry.get(water_voxel_id);
        assert_eq!(def.transparency, Transparency::SemiTransparent);
    }

    #[test]
    fn test_fluid_is_non_solid() {
        let mut voxel_registry = VoxelTypeRegistry::new();
        let mut fluid_registry = FluidTypeRegistry::new();

        let lava_fluid_id = fluid_registry.register(FluidTypeDef {
            name: "lava".to_string(),
            viscosity: 50.0,
            color: [1.0, 0.3, 0.0, 0.95],
            flow_speed: 0.1,
            density: 3100.0,
            damage_per_second: 20.0,
            light_emission: 14,
        }).unwrap();

        let lava_voxel_id = voxel_registry.register(VoxelTypeDef {
            name: "lava".to_string(),
            solid: false,
            transparency: Transparency::SemiTransparent,
            material_index: 1,
            light_emission: 14,
            fluid_id: Some(lava_fluid_id),
        }).unwrap();

        let def = voxel_registry.get(lava_voxel_id);
        assert!(!def.solid, "Fluid voxel must be non-solid");
    }

    #[test]
    fn test_viscosity_varies_by_fluid_type() {
        let mut registry = FluidTypeRegistry::new();

        let water_id = registry.register(FluidTypeDef {
            name: "water".to_string(),
            viscosity: 1.0,
            color: [0.2, 0.4, 0.9, 0.6],
            flow_speed: 1.0,
            density: 1000.0,
            damage_per_second: 0.0,
            light_emission: 0,
        }).unwrap();

        let lava_id = registry.register(FluidTypeDef {
            name: "lava".to_string(),
            viscosity: 50.0,
            color: [1.0, 0.3, 0.0, 0.95],
            flow_speed: 0.1,
            density: 3100.0,
            damage_per_second: 20.0,
            light_emission: 14,
        }).unwrap();

        let oil_id = registry.register(FluidTypeDef {
            name: "oil".to_string(),
            viscosity: 5.0,
            color: [0.15, 0.1, 0.05, 0.8],
            flow_speed: 0.5,
            density: 800.0,
            damage_per_second: 0.0,
            light_emission: 0,
        }).unwrap();

        assert!(
            registry.get(water_id).viscosity < registry.get(oil_id).viscosity,
            "Water should be less viscous than oil"
        );
        assert!(
            registry.get(oil_id).viscosity < registry.get(lava_id).viscosity,
            "Oil should be less viscous than lava"
        );
    }

    #[test]
    fn test_fluid_registry_lookup_by_name() {
        let mut registry = FluidTypeRegistry::new();
        registry.register(FluidTypeDef {
            name: "water".to_string(),
            viscosity: 1.0,
            color: [0.2, 0.4, 0.9, 0.6],
            flow_speed: 1.0,
            density: 1000.0,
            damage_per_second: 0.0,
            light_emission: 0,
        }).unwrap();

        assert!(registry.lookup_by_name("water").is_some());
        assert!(registry.lookup_by_name("nonexistent").is_none());
    }

    #[test]
    fn test_duplicate_fluid_name_rejected() {
        let mut registry = FluidTypeRegistry::new();
        registry.register(FluidTypeDef {
            name: "water".to_string(),
            viscosity: 1.0,
            color: [0.2, 0.4, 0.9, 0.6],
            flow_speed: 1.0,
            density: 1000.0,
            damage_per_second: 0.0,
            light_emission: 0,
        }).unwrap();

        let result = registry.register(FluidTypeDef {
            name: "water".to_string(),
            viscosity: 2.0,
            color: [0.0; 4],
            flow_speed: 1.0,
            density: 1000.0,
            damage_per_second: 0.0,
            light_emission: 0,
        });
        assert!(matches!(result, Err(RegistryError::DuplicateName)));
    }
}
```
