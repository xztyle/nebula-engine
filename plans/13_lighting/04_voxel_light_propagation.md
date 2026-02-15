# Voxel Light Propagation

## Problem

Shadow maps handle directional light from the sun, but they cannot illuminate the interiors of caves, tunnels, or underground structures where players place torches and lanterns. Voxel worlds need a block-level lighting model where each voxel stores a discrete light level and light "floods" outward from sources through transparent blocks, decaying with distance and being stopped by opaque geometry. Without this, underground areas would be pitch black (no sun reaches them) and placed light sources would have no visible effect on the surrounding voxels. The system must handle two distinct light channels: sunlight (propagates downward from the sky with no decay, horizontally with decay) and block light (propagates equally in all 6 directions from emissive voxels). It must also be incremental — when a torch is placed or a wall is broken, only the affected region re-propagates, not the entire world.

## Solution

### Light Level Storage

Each voxel stores two 4-bit light levels packed into a single byte:

```rust
/// Packed light value: high nibble = sunlight, low nibble = block light.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VoxelLight(pub u8);

impl VoxelLight {
    pub const MAX_LEVEL: u8 = 15;

    pub fn sunlight(self) -> u8 {
        (self.0 >> 4) & 0xF
    }

    pub fn block_light(self) -> u8 {
        self.0 & 0xF
    }

    pub fn set_sunlight(&mut self, level: u8) {
        debug_assert!(level <= 15);
        self.0 = (self.0 & 0x0F) | (level << 4);
    }

    pub fn set_block_light(&mut self, level: u8) {
        debug_assert!(level <= 15);
        self.0 = (self.0 & 0xF0) | (level & 0x0F);
    }
}
```

A chunk of 32x32x32 voxels requires 32,768 bytes for light data — one byte per voxel.

### Light Data in Chunks

```rust
pub struct ChunkLightMap {
    /// 32x32x32 = 32,768 entries.
    data: Box<[VoxelLight; 32 * 32 * 32]>,
}

impl ChunkLightMap {
    pub fn new_dark() -> Self {
        Self {
            data: Box::new([VoxelLight(0); 32 * 32 * 32]),
        }
    }

    pub fn get(&self, x: u32, y: u32, z: u32) -> VoxelLight {
        self.data[Self::index(x, y, z)]
    }

    pub fn set(&mut self, x: u32, y: u32, z: u32, light: VoxelLight) {
        self.data[Self::index(x, y, z)] = light;
    }

    fn index(x: u32, y: u32, z: u32) -> usize {
        (y * 32 * 32 + z * 32 + x) as usize
    }
}
```

### Sunlight Propagation

Sunlight enters from the top of the world (or the top face of the cubesphere). It propagates downward through transparent/air voxels with no decay (stays at level 15). When it encounters open horizontal space, it spreads horizontally with -1 per step, behaving like block light in the horizontal plane.

```rust
pub fn propagate_sunlight(chunk: &mut ChunkLightMap, voxels: &ChunkVoxels) {
    // Phase 1: Vertical propagation (column-wise).
    for x in 0..32 {
        for z in 0..32 {
            let mut sun_level = 15u8; // Start at max if exposed to sky.
            for y in (0..32).rev() { // Top to bottom.
                if voxels.is_opaque(x, y, z) {
                    sun_level = 0;
                } else {
                    chunk.set(x, y, z, {
                        let mut l = chunk.get(x, y, z);
                        l.set_sunlight(sun_level);
                        l
                    });
                }
            }
        }
    }
    // Phase 2: Horizontal spread via BFS (same as block light).
    propagate_bfs(chunk, voxels, LightChannel::Sun);
}
```

### Block Light Propagation (Flood-Fill BFS)

When a light source is placed (or a chunk is first loaded), block light propagates via a breadth-first flood fill:

```rust
use std::collections::VecDeque;

pub fn propagate_block_light(
    chunk: &mut ChunkLightMap,
    voxels: &ChunkVoxels,
    sources: &[(u32, u32, u32, u8)], // (x, y, z, level)
) {
    let mut queue = VecDeque::new();

    // Seed the queue with all light sources.
    for &(x, y, z, level) in sources {
        let mut l = chunk.get(x, y, z);
        l.set_block_light(level);
        chunk.set(x, y, z, l);
        queue.push_back((x, y, z));
    }

    // BFS: spread in 6 directions with -1 per step.
    while let Some((x, y, z)) = queue.pop_front() {
        let current_level = chunk.get(x, y, z).block_light();
        if current_level <= 1 { continue; }
        let new_level = current_level - 1;

        for (dx, dy, dz) in NEIGHBORS_6 {
            let (nx, ny, nz) = (x as i32 + dx, y as i32 + dy, z as i32 + dz);
            if !in_bounds(nx, ny, nz) { continue; } // cross-chunk handled in story 07
            let (nx, ny, nz) = (nx as u32, ny as u32, nz as u32);
            if voxels.is_opaque(nx, ny, nz) { continue; }
            if chunk.get(nx, ny, nz).block_light() >= new_level { continue; }
            let mut l = chunk.get(nx, ny, nz);
            l.set_block_light(new_level);
            chunk.set(nx, ny, nz, l);
            queue.push_back((nx, ny, nz));
        }
    }
}

const NEIGHBORS_6: [(i32, i32, i32); 6] = [
    (1, 0, 0), (-1, 0, 0),
    (0, 1, 0), (0, -1, 0),
    (0, 0, 1), (0, 0, -1),
];
```

### Light Removal

When a light source is removed (torch broken), a reverse BFS clears the old light values, then re-propagation fills from remaining sources:

```rust
pub fn remove_block_light(
    chunk: &mut ChunkLightMap,
    voxels: &ChunkVoxels,
    x: u32, y: u32, z: u32,
) {
    // BFS removal: collect all voxels that were lit by this source.
    // Then re-propagate from any remaining sources in the affected region.
}
```

### Transparent Blocks

Blocks with `Transparency::SemiTransparent` (e.g., glass, water) transmit light but may optionally reduce it by an extra -1 per step. Blocks with `Transparency::FullyTransparent` (air) transmit without extra penalty. Blocks with `Transparency::Opaque` block propagation entirely.

## Outcome

A `ChunkLightMap` and flood-fill propagation system in `nebula_lighting` that computes per-voxel light levels for both sunlight and block light channels. Light levels range from 0 (dark) to 15 (maximum). The meshing system reads these values to interpolate vertex lighting. Running `cargo test -p nebula_lighting` passes all voxel light propagation tests. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Light floods into caves from the surface entrance. The light level decreases with distance from the opening, creating a natural falloff into darkness.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_voxel` | workspace | `ChunkVoxels`, `VoxelTypeRegistry`, transparency queries |

No external crates required. Light propagation is pure Rust arithmetic and BFS over a fixed-size 3D array.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_empty_chunk() -> (ChunkLightMap, ChunkVoxels) {
        (ChunkLightMap::new_dark(), ChunkVoxels::new_air())
    }

    fn make_chunk_with_floor(floor_y: u32) -> (ChunkLightMap, ChunkVoxels) {
        let mut voxels = ChunkVoxels::new_air();
        for x in 0..32 {
            for z in 0..32 {
                voxels.set(x, floor_y, z, VoxelTypeId::STONE);
            }
        }
        (ChunkLightMap::new_dark(), voxels)
    }

    #[test]
    fn test_sunlight_fills_open_area_to_max() {
        let (mut light, voxels) = make_empty_chunk();
        propagate_sunlight(&mut light, &voxels);
        // Every voxel in a fully open chunk should have sunlight = 15.
        for y in 0..32 {
            assert_eq!(
                light.get(16, y, 16).sunlight(),
                15,
                "open area at y={y} should have max sunlight"
            );
        }
    }

    #[test]
    fn test_block_light_decays_with_distance() {
        let (mut light, voxels) = make_empty_chunk();
        // Place a level-15 light source at (16, 16, 16).
        propagate_block_light(&mut light, &voxels, &[(16, 16, 16, 15)]);
        // At distance 1, level should be 14.
        assert_eq!(light.get(17, 16, 16).block_light(), 14);
        // At distance 5, level should be 10.
        assert_eq!(light.get(21, 16, 16).block_light(), 10);
        // At distance 15, level should be 0.
        assert_eq!(light.get(31, 16, 16).block_light(), 0);
    }

    #[test]
    fn test_opaque_block_creates_shadow() {
        let (mut light, mut voxels) = make_empty_chunk();
        // Place an opaque block between the source and the test point.
        voxels.set(18, 16, 16, VoxelTypeId::STONE);
        propagate_block_light(&mut light, &voxels, &[(16, 16, 16, 15)]);
        // The voxel directly behind the wall should NOT receive direct propagation
        // along the X axis (it might get some from wrapping around).
        let behind_wall = light.get(19, 16, 16).block_light();
        let without_wall_equivalent = 15 - 3; // distance 3 from source = 12
        assert!(
            behind_wall < without_wall_equivalent,
            "block behind wall should have less light ({behind_wall}) than open path ({without_wall_equivalent})"
        );
    }

    #[test]
    fn test_transparent_block_transmits_light() {
        let (mut light, mut voxels) = make_empty_chunk();
        // Place a transparent block (glass) between source and test point.
        voxels.set(17, 16, 16, VoxelTypeId::GLASS); // semi-transparent
        propagate_block_light(&mut light, &voxels, &[(16, 16, 16, 15)]);
        // Light should pass through the transparent block.
        let through_glass = light.get(18, 16, 16).block_light();
        assert!(through_glass > 0, "light should pass through transparent block");
    }

    #[test]
    fn test_light_level_in_valid_range() {
        let (mut light, voxels) = make_empty_chunk();
        propagate_block_light(&mut light, &voxels, &[(16, 16, 16, 15)]);
        for x in 0..32 {
            for y in 0..32 {
                for z in 0..32 {
                    let bl = light.get(x, y, z).block_light();
                    let sl = light.get(x, y, z).sunlight();
                    assert!(bl <= VoxelLight::MAX_LEVEL, "block light {bl} exceeds max");
                    assert!(sl <= VoxelLight::MAX_LEVEL, "sunlight {sl} exceeds max");
                }
            }
        }
    }

    #[test]
    fn test_propagation_handles_chunk_boundaries() {
        // When BFS reaches a chunk edge, it should stop without panic.
        // Cross-chunk propagation is handled in story 07; here we verify
        // the single-chunk BFS terminates cleanly at boundaries.
        let (mut light, voxels) = make_empty_chunk();
        // Place light at corner (0, 0, 0).
        propagate_block_light(&mut light, &voxels, &[(0, 0, 0, 15)]);
        // Should not panic, and the light at (0,0,0) should be 15.
        assert_eq!(light.get(0, 0, 0).block_light(), 15);
        // Light should propagate inward but not wrap around.
        assert_eq!(light.get(1, 0, 0).block_light(), 14);
    }
}
```
