# Cellular Automaton Rules

## Problem

Fluid in the voxel world needs to move. A lake at the top of a mountain must flow downhill, water poured into a cave must fill the floor, and a river must spread across flat terrain. Real-time Navier-Stokes simulation is far too expensive for a voxel world with millions of active fluid cells across a cubesphere planet. The engine needs a simple, deterministic, parallelizable fluid propagation model that produces visually convincing behavior at game-relevant scales. The cellular automaton approach (used successfully by Minecraft, Dwarf Fortress, and other voxel games) offers exactly this: local rules applied to each fluid cell that produce emergent large-scale flow.

## Solution

### Fluid Simulation Schedule

Fluid updates run in the `FixedUpdate` schedule alongside physics, at 60 Hz. However, not every fluid cell updates every tick. A fluid cell's update frequency is governed by its type's `flow_speed` property:

```rust
/// Determines how many FixedUpdate ticks between updates for this fluid.
/// tick_interval = max(1, (1.0 / flow_speed).round() as u32)
/// Water (flow_speed=1.0): every tick
/// Oil (flow_speed=0.5): every 2 ticks
/// Lava (flow_speed=0.1): every 10 ticks
fn tick_interval(flow_speed: f32) -> u32 {
    (1.0 / flow_speed).round().max(1.0) as u32
}
```

### Cellular Automaton Rules

Each active fluid cell applies these rules in priority order during its update:

**Rule 1 — Flow Down (Gravity)**

If the voxel directly below (toward planet center, see story 03) is air or contains the same fluid at a lower level, transfer as much fluid as possible downward:

```rust
fn rule_flow_down(cell: &mut FluidState, below: &mut FluidState) -> bool {
    if cell.is_empty() { return false; }

    let space_below = if below.is_empty() {
        FluidState::FULL
    } else if below.fluid_type == cell.fluid_type && below.level < FluidState::FULL {
        FluidState::FULL - below.level
    } else {
        return false; // Different fluid or full — blocked
    };

    let transfer = cell.level.min(space_below);
    cell.level -= transfer;
    if below.is_empty() {
        below.fluid_type = cell.fluid_type;
    }
    below.level += transfer;
    true
}
```

**Rule 2 — Spread Horizontally**

If the cell cannot flow further down (below is solid or full), spread to horizontal neighbors (the 4 lateral neighbors on the cubesphere surface). Fluid flows to the neighbor with the lowest level, equalizing:

```rust
fn rule_spread_horizontal(
    cell: &mut FluidState,
    neighbors: &mut [FluidState; 4], // N, S, E, W
) -> bool {
    if cell.level <= 1 { return false; } // Level 1 doesn't spread further

    // Find neighbors that are air or same fluid with lower level
    let mut targets: Vec<(usize, u8)> = neighbors.iter().enumerate()
        .filter_map(|(i, n)| {
            if n.is_empty() || (n.fluid_type == cell.fluid_type && n.level < cell.level) {
                Some((i, n.level))
            } else {
                None
            }
        })
        .collect();

    if targets.is_empty() { return false; }

    // Sort by level ascending — fill lowest neighbors first
    targets.sort_by_key(|&(_, level)| level);

    // Equalize: compute the average level across cell + targets
    let total: u16 = cell.level as u16
        + targets.iter().map(|&(_, l)| l as u16).sum::<u16>();
    let count = 1 + targets.len() as u16;
    let base_level = (total / count) as u8;
    let remainder = (total % count) as u8;

    cell.level = base_level + if remainder > 0 { 1 } else { 0 };
    for (idx, (i, _)) in targets.iter().enumerate() {
        let extra = if (idx as u8 + 1) < remainder { 1 } else { 0 };
        if neighbors[*i].is_empty() {
            neighbors[*i].fluid_type = cell.fluid_type;
        }
        neighbors[*i].level = base_level + extra;
    }
    true
}
```

**Rule 3 — Equalize**

Adjacent cells of the same fluid at the same level do nothing (equilibrium). This rule is implicit: if neither Rule 1 nor Rule 2 transfers any fluid, the cell is stable and is removed from the active set until a neighbor changes.

### Source Blocks

A source block is a special voxel component that generates fluid indefinitely:

```rust
#[derive(Clone, Debug)]
pub struct FluidSource {
    pub fluid_type: FluidTypeId,
    /// How many levels of fluid to generate per tick.
    pub generation_rate: u8,
}
```

Each tick, a source block sets its own cell to level 7 (full). This creates a continuous stream of fluid that flows outward following the automaton rules. Source blocks are placed by terrain generation (springs, rivers) or by gameplay systems.

### Batched Chunk Updates

Updating every fluid cell every tick is too expensive for a planet-scale world. The system maintains an **active fluid set** — a set of chunk addresses that contain at least one fluid cell that changed in the last tick:

```rust
pub struct FluidSimulation {
    /// Chunks that need fluid updates this tick.
    active_chunks: HashSet<ChunkAddress>,
    /// Chunks to activate next tick (due to propagation from neighbors).
    pending_chunks: HashSet<ChunkAddress>,
    /// Maximum chunks to process per tick (budget).
    max_chunks_per_tick: usize,
    /// Current simulation tick counter.
    tick: u64,
}
```

Each tick, the system processes up to `max_chunks_per_tick` active chunks. Within each chunk, only cells flagged as dirty (their level changed or a neighbor changed) are updated. After processing, if any cell at a chunk boundary changed, the neighboring chunk is added to `pending_chunks`.

### ECS System

```rust
fn fluid_simulation_system(
    mut simulation: ResMut<FluidSimulation>,
    mut chunks: Query<(&ChunkAddress, &mut ChunkData, &mut ChunkFluidData)>,
    fluid_registry: Res<Arc<FluidTypeRegistry>>,
    sources: Query<(&VoxelPosition, &FluidSource)>,
) {
    // 1. Apply source blocks
    for (pos, source) in sources.iter() {
        simulation.apply_source(pos, source, &mut chunks);
    }

    // 2. Process active chunks (batched)
    let budget = simulation.max_chunks_per_tick;
    let active: Vec<_> = simulation.active_chunks.iter().copied().take(budget).collect();
    for addr in &active {
        simulation.update_chunk(addr, &mut chunks, &fluid_registry);
    }

    // 3. Rotate pending into active for next tick
    simulation.active_chunks = std::mem::take(&mut simulation.pending_chunks);
    simulation.tick += 1;
}
```

### Determinism

The update order within a chunk is fixed: iterate cells in Morton (Z-order) curve order. Between chunks, process in `ChunkAddress` sort order. This ensures deterministic simulation regardless of hash map iteration order, which is critical for multiplayer synchronization.

## Outcome

The `nebula-fluid` crate exports `FluidSimulation`, `FluidSource`, and the `fluid_simulation_system`. Fluid cells obey cellular automaton rules: flow down under gravity, spread horizontally to equalize, and reach equilibrium. Source blocks generate fluid continuously. The system processes a budgeted number of chunks per tick and maintains an active set for efficiency. Running `cargo test -p nebula-fluid` passes all cellular automaton tests. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Water flows to adjacent empty or lower-level voxels using cellular automaton rules. A water source on a hillside produces a cascading waterfall.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | ECS framework for systems, resources, queries, and `FixedUpdate` schedule |
| `hashbrown` | `0.15` | Fast `HashSet` for active/pending chunk tracking |
| `smallvec` | `1.15` | Stack-allocated neighbor lists to avoid per-cell heap allocation |

Depends on Epic 06 (`nebula-voxel` chunk data), Epic 05 (chunk neighbor system), and story 01 of this epic (`FluidState`, `FluidTypeId`).

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_fluid(level: u8) -> FluidState {
        FluidState::new(FluidTypeId(0), level)
    }

    fn air() -> FluidState {
        FluidState { fluid_type: FluidTypeId(0), level: 0 }
    }

    #[test]
    fn test_water_flows_down() {
        let mut cell = make_fluid(7);
        let mut below = air();
        let changed = rule_flow_down(&mut cell, &mut below);
        assert!(changed, "Fluid should flow into empty space below");
        assert_eq!(below.level, 7, "All fluid should transfer down");
        assert_eq!(cell.level, 0, "Source cell should be empty after full transfer");
    }

    #[test]
    fn test_water_flows_down_partial() {
        let mut cell = make_fluid(4);
        let mut below = make_fluid(5); // 2 units of space
        let changed = rule_flow_down(&mut cell, &mut below);
        assert!(changed);
        assert_eq!(below.level, 7, "Below should be full (5 + 2)");
        assert_eq!(cell.level, 2, "Cell should have 2 remaining (4 - 2)");
    }

    #[test]
    fn test_water_spreads_horizontally_on_flat_surface() {
        let mut cell = make_fluid(6);
        let mut neighbors = [air(), air(), air(), air()];
        let changed = rule_spread_horizontal(&mut cell, &mut neighbors);
        assert!(changed, "Fluid should spread to empty horizontal neighbors");
        // Total = 6, count = 5 (cell + 4 neighbors), base = 1, remainder = 1
        // Cell gets base+1=2, rest get base=1... but total should remain 6
        let total: u8 = cell.level + neighbors.iter().map(|n| n.level).sum::<u8>();
        assert_eq!(total, 6, "Fluid volume must be conserved");
    }

    #[test]
    fn test_water_seeks_lowest_point() {
        // Simulate multiple ticks: fluid above should flow down, not stay level
        let mut column = [air(), air(), air(), make_fluid(7)]; // [bottom..top], fluid at top

        // Simulate gravity flow for each cell from top to bottom
        for _ in 0..4 {
            for i in (1..column.len()).rev() {
                let (lower, upper) = column.split_at_mut(i);
                rule_flow_down(&mut upper[0], &mut lower[i - 1]);
            }
        }

        assert_eq!(column[0].level, 7, "Fluid should have reached the bottom");
        assert_eq!(column[3].level, 0, "Top cell should be empty");
    }

    #[test]
    fn test_source_block_generates_fluid() {
        let source = FluidSource {
            fluid_type: FluidTypeId(0),
            generation_rate: 7,
        };

        let mut cell = air();
        // Simulate source application
        cell.fluid_type = source.fluid_type;
        cell.level = source.generation_rate.min(FluidState::FULL);

        assert_eq!(cell.level, 7, "Source should fill cell to max");
        assert_eq!(cell.fluid_type, FluidTypeId(0));
    }

    #[test]
    fn test_fluid_stabilizes_at_equilibrium() {
        // 5 cells in a row, center has level 5, rest have level 5
        let level = 5;
        let mut cell = make_fluid(level);
        let mut neighbors = [
            make_fluid(level),
            make_fluid(level),
            make_fluid(level),
            make_fluid(level),
        ];

        let changed = rule_spread_horizontal(&mut cell, &mut neighbors);
        assert!(!changed, "Equal-level cells should not exchange fluid");

        assert_eq!(cell.level, level);
        for n in &neighbors {
            assert_eq!(n.level, level, "Neighbor levels should be unchanged");
        }
    }

    #[test]
    fn test_fluid_does_not_flow_into_different_fluid() {
        let mut water = FluidState::new(FluidTypeId(0), 7); // water
        let mut lava = FluidState::new(FluidTypeId(1), 3);  // lava
        let changed = rule_flow_down(&mut water, &mut lava);
        assert!(!changed, "Water should not flow into a cell occupied by lava");
        assert_eq!(water.level, 7);
        assert_eq!(lava.level, 3);
    }

    #[test]
    fn test_volume_conservation_horizontal_spread() {
        for initial_level in 2..=7 {
            let mut cell = make_fluid(initial_level);
            let mut neighbors = [air(), air(), air(), air()];
            rule_spread_horizontal(&mut cell, &mut neighbors);
            let total: u8 = cell.level + neighbors.iter().map(|n| n.level).sum::<u8>();
            assert_eq!(
                total, initial_level,
                "Volume must be conserved for initial level {initial_level}"
            );
        }
    }

    #[test]
    fn test_tick_interval_matches_flow_speed() {
        assert_eq!(tick_interval(1.0), 1);   // Water: every tick
        assert_eq!(tick_interval(0.5), 2);   // Oil: every 2 ticks
        assert_eq!(tick_interval(0.1), 10);  // Lava: every 10 ticks
    }
}
```
