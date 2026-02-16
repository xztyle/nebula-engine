//! Per-voxel light storage and flood-fill propagation for sunlight and block light.
//!
//! Each voxel stores two 4-bit light levels packed into a single byte:
//! the high nibble for sunlight and the low nibble for block (emissive) light.
//! Light propagates via BFS with -1 decay per step, blocked by opaque voxels.

use std::collections::VecDeque;

use nebula_voxel::{CHUNK_SIZE, ChunkData, Transparency, VoxelTypeRegistry};

/// Packed light value: high nibble = sunlight, low nibble = block light.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VoxelLight(pub u8);

impl VoxelLight {
    /// Maximum light level for either channel.
    pub const MAX_LEVEL: u8 = 15;

    /// Returns the sunlight level (0–15).
    pub fn sunlight(self) -> u8 {
        (self.0 >> 4) & 0xF
    }

    /// Returns the block light level (0–15).
    pub fn block_light(self) -> u8 {
        self.0 & 0xF
    }

    /// Sets the sunlight level (0–15).
    pub fn set_sunlight(&mut self, level: u8) {
        debug_assert!(level <= 15);
        self.0 = (self.0 & 0x0F) | (level << 4);
    }

    /// Sets the block light level (0–15).
    pub fn set_block_light(&mut self, level: u8) {
        debug_assert!(level <= 15);
        self.0 = (self.0 & 0xF0) | (level & 0x0F);
    }
}

/// Per-voxel light data for a 32×32×32 chunk.
pub struct ChunkLightMap {
    /// One byte per voxel: 32×32×32 = 32 768 entries.
    data: Box<[VoxelLight; CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE]>,
}

impl ChunkLightMap {
    /// Creates a fully dark light map (all zeros).
    pub fn new_dark() -> Self {
        Self {
            data: Box::new([VoxelLight(0); CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE]),
        }
    }

    /// Returns the light value at `(x, y, z)`. Each coordinate must be in `0..32`.
    pub fn get(&self, x: u32, y: u32, z: u32) -> VoxelLight {
        self.data[Self::index(x, y, z)]
    }

    /// Sets the light value at `(x, y, z)`.
    pub fn set(&mut self, x: u32, y: u32, z: u32, light: VoxelLight) {
        self.data[Self::index(x, y, z)] = light;
    }

    fn index(x: u32, y: u32, z: u32) -> usize {
        (y * (CHUNK_SIZE as u32) * (CHUNK_SIZE as u32) + z * (CHUNK_SIZE as u32) + x) as usize
    }
}

/// The six axis-aligned neighbour offsets.
const NEIGHBORS_6: [(i32, i32, i32); 6] = [
    (1, 0, 0),
    (-1, 0, 0),
    (0, 1, 0),
    (0, -1, 0),
    (0, 0, 1),
    (0, 0, -1),
];

fn in_bounds(x: i32, y: i32, z: i32) -> bool {
    let s = CHUNK_SIZE as i32;
    (0..s).contains(&x) && (0..s).contains(&y) && (0..s).contains(&z)
}

/// Returns `true` if the voxel at `(x, y, z)` is opaque according to the registry.
fn is_opaque(voxels: &ChunkData, registry: &VoxelTypeRegistry, x: u32, y: u32, z: u32) -> bool {
    let id = voxels.get(x as usize, y as usize, z as usize);
    !registry.is_transparent(id)
}

/// Returns extra attenuation for semi-transparent blocks (1 extra decay).
fn extra_decay(voxels: &ChunkData, registry: &VoxelTypeRegistry, x: u32, y: u32, z: u32) -> u8 {
    let id = voxels.get(x as usize, y as usize, z as usize);
    let def = registry.get(id);
    if def.transparency == Transparency::SemiTransparent {
        1
    } else {
        0
    }
}

/// Horizontal BFS spread for the sunlight channel.
fn propagate_sunlight_bfs(
    chunk: &mut ChunkLightMap,
    voxels: &ChunkData,
    registry: &VoxelTypeRegistry,
) {
    let s = CHUNK_SIZE as u32;
    let mut queue = VecDeque::new();

    // Seed queue with all voxels that have sunlight > 1.
    for y in 0..s {
        for z in 0..s {
            for x in 0..s {
                if chunk.get(x, y, z).sunlight() > 1 {
                    queue.push_back((x, y, z));
                }
            }
        }
    }

    while let Some((x, y, z)) = queue.pop_front() {
        let current = chunk.get(x, y, z).sunlight();
        if current <= 1 {
            continue;
        }

        for (dx, dy, dz) in NEIGHBORS_6 {
            let (nx, ny, nz) = (x as i32 + dx, y as i32 + dy, z as i32 + dz);
            if !in_bounds(nx, ny, nz) {
                continue;
            }
            let (nx, ny, nz) = (nx as u32, ny as u32, nz as u32);
            if is_opaque(voxels, registry, nx, ny, nz) {
                continue;
            }
            let decay = 1 + extra_decay(voxels, registry, nx, ny, nz);
            if current <= decay {
                continue;
            }
            let new_level = current - decay;

            if chunk.get(nx, ny, nz).sunlight() >= new_level {
                continue;
            }

            let mut l = chunk.get(nx, ny, nz);
            l.set_sunlight(new_level);
            chunk.set(nx, ny, nz, l);
            queue.push_back((nx, ny, nz));
        }
    }
}

/// Propagates sunlight from the top of the chunk downward, then spreads horizontally.
///
/// Sunlight enters at level 15 from the top face and propagates straight down
/// through transparent voxels without decay. Horizontal spread uses BFS with -1
/// per step, identical to block light.
pub fn propagate_sunlight(
    chunk: &mut ChunkLightMap,
    voxels: &ChunkData,
    registry: &VoxelTypeRegistry,
) {
    let s = CHUNK_SIZE as u32;

    // Phase 1: vertical propagation (column-wise, top to bottom).
    for x in 0..s {
        for z in 0..s {
            let mut sun_level: u8 = 15;
            for y in (0..s).rev() {
                if is_opaque(voxels, registry, x, y, z) {
                    sun_level = 0;
                } else {
                    let mut l = chunk.get(x, y, z);
                    l.set_sunlight(sun_level);
                    chunk.set(x, y, z, l);
                }
            }
        }
    }

    // Phase 2: horizontal BFS spread.
    propagate_sunlight_bfs(chunk, voxels, registry);
}

/// Propagates block light from the given sources via BFS flood-fill.
///
/// Each source is `(x, y, z, level)`. Light decays by 1 per step through
/// fully transparent blocks, and by 2 through semi-transparent blocks.
/// Opaque blocks stop propagation entirely.
pub fn propagate_block_light(
    chunk: &mut ChunkLightMap,
    voxels: &ChunkData,
    registry: &VoxelTypeRegistry,
    sources: &[(u32, u32, u32, u8)],
) {
    let mut queue = VecDeque::new();

    // Seed sources.
    for &(x, y, z, level) in sources {
        let mut l = chunk.get(x, y, z);
        l.set_block_light(level);
        chunk.set(x, y, z, l);
        queue.push_back((x, y, z));
    }

    // BFS flood-fill.
    while let Some((x, y, z)) = queue.pop_front() {
        let current_level = chunk.get(x, y, z).block_light();
        if current_level <= 1 {
            continue;
        }

        for (dx, dy, dz) in NEIGHBORS_6 {
            let (nx, ny, nz) = (x as i32 + dx, y as i32 + dy, z as i32 + dz);
            if !in_bounds(nx, ny, nz) {
                continue;
            }
            let (nx, ny, nz) = (nx as u32, ny as u32, nz as u32);
            if is_opaque(voxels, registry, nx, ny, nz) {
                continue;
            }
            let decay = 1 + extra_decay(voxels, registry, nx, ny, nz);
            if current_level <= decay {
                continue;
            }
            let new_level = current_level - decay;
            if chunk.get(nx, ny, nz).block_light() >= new_level {
                continue;
            }
            let mut l = chunk.get(nx, ny, nz);
            l.set_block_light(new_level);
            chunk.set(nx, ny, nz, l);
            queue.push_back((nx, ny, nz));
        }
    }
}

/// Removes block light originating from `(x, y, z)` via reverse BFS, then
/// re-propagates from any remaining sources in the affected region.
pub fn remove_block_light(
    chunk: &mut ChunkLightMap,
    voxels: &ChunkData,
    registry: &VoxelTypeRegistry,
    x: u32,
    y: u32,
    z: u32,
) {
    let old_level = chunk.get(x, y, z).block_light();
    if old_level == 0 {
        return;
    }

    // Phase 1: reverse BFS to clear light that was propagated from this source.
    let mut remove_queue: VecDeque<(u32, u32, u32, u8)> = VecDeque::new();
    let mut relight_queue: Vec<(u32, u32, u32, u8)> = Vec::new();

    // Zero the source.
    let mut l = chunk.get(x, y, z);
    l.set_block_light(0);
    chunk.set(x, y, z, l);
    remove_queue.push_back((x, y, z, old_level));

    while let Some((rx, ry, rz, level)) = remove_queue.pop_front() {
        for (dx, dy, dz) in NEIGHBORS_6 {
            let (nx, ny, nz) = (rx as i32 + dx, ry as i32 + dy, rz as i32 + dz);
            if !in_bounds(nx, ny, nz) {
                continue;
            }
            let (nx, ny, nz) = (nx as u32, ny as u32, nz as u32);
            let neighbor_level = chunk.get(nx, ny, nz).block_light();
            if neighbor_level == 0 {
                continue;
            }
            if neighbor_level < level {
                // This was propagated from the removed source — clear it.
                let mut nl = chunk.get(nx, ny, nz);
                nl.set_block_light(0);
                chunk.set(nx, ny, nz, nl);
                remove_queue.push_back((nx, ny, nz, neighbor_level));
            } else {
                // This is from another source — re-propagate from here.
                relight_queue.push((nx, ny, nz, neighbor_level));
            }
        }
    }

    // Phase 2: re-propagate from boundary sources.
    if !relight_queue.is_empty() {
        propagate_block_light(chunk, voxels, registry, &relight_queue);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_voxel::{Transparency, VoxelTypeDef, VoxelTypeId, VoxelTypeRegistry};

    /// Creates a registry with air(0), stone(1), glass(2).
    fn test_registry() -> VoxelTypeRegistry {
        let mut reg = VoxelTypeRegistry::new();
        reg.register(VoxelTypeDef {
            name: "stone".to_string(),
            solid: true,
            transparency: Transparency::Opaque,
            material_index: 1,
            light_emission: 0,
        })
        .unwrap();
        reg.register(VoxelTypeDef {
            name: "glass".to_string(),
            solid: true,
            transparency: Transparency::SemiTransparent,
            material_index: 2,
            light_emission: 0,
        })
        .unwrap();
        reg
    }

    fn make_empty_chunk() -> (ChunkLightMap, ChunkData) {
        (ChunkLightMap::new_dark(), ChunkData::new_air())
    }

    #[test]
    fn test_sunlight_fills_open_area_to_max() {
        let reg = test_registry();
        let (mut light, voxels) = make_empty_chunk();
        propagate_sunlight(&mut light, &voxels, &reg);
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
        let reg = test_registry();
        let (mut light, voxels) = make_empty_chunk();
        propagate_block_light(&mut light, &voxels, &reg, &[(16, 16, 16, 15)]);
        assert_eq!(light.get(17, 16, 16).block_light(), 14);
        assert_eq!(light.get(21, 16, 16).block_light(), 10);
        assert_eq!(light.get(31, 16, 16).block_light(), 0);
    }

    #[test]
    fn test_opaque_block_creates_shadow() {
        let reg = test_registry();
        let (mut light, mut voxels) = make_empty_chunk();
        // stone = VoxelTypeId(1)
        voxels.set(18, 16, 16, VoxelTypeId(1));
        propagate_block_light(&mut light, &voxels, &reg, &[(16, 16, 16, 15)]);
        let behind_wall = light.get(19, 16, 16).block_light();
        let without_wall_equivalent = 15 - 3; // distance 3 = 12
        assert!(
            behind_wall < without_wall_equivalent,
            "block behind wall should have less light ({behind_wall}) than open path ({without_wall_equivalent})"
        );
    }

    #[test]
    fn test_transparent_block_transmits_light() {
        let reg = test_registry();
        let (mut light, mut voxels) = make_empty_chunk();
        // glass = VoxelTypeId(2)
        voxels.set(17, 16, 16, VoxelTypeId(2));
        propagate_block_light(&mut light, &voxels, &reg, &[(16, 16, 16, 15)]);
        let through_glass = light.get(18, 16, 16).block_light();
        assert!(
            through_glass > 0,
            "light should pass through transparent block"
        );
    }

    #[test]
    fn test_light_level_in_valid_range() {
        let reg = test_registry();
        let (mut light, voxels) = make_empty_chunk();
        propagate_block_light(&mut light, &voxels, &reg, &[(16, 16, 16, 15)]);
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
        let reg = test_registry();
        let (mut light, voxels) = make_empty_chunk();
        propagate_block_light(&mut light, &voxels, &reg, &[(0, 0, 0, 15)]);
        assert_eq!(light.get(0, 0, 0).block_light(), 15);
        assert_eq!(light.get(1, 0, 0).block_light(), 14);
    }

    #[test]
    fn test_remove_block_light_clears_and_relights() {
        let reg = test_registry();
        let (mut light, voxels) = make_empty_chunk();
        propagate_block_light(&mut light, &voxels, &reg, &[(16, 16, 16, 15)]);
        assert_eq!(light.get(17, 16, 16).block_light(), 14);

        remove_block_light(&mut light, &voxels, &reg, 16, 16, 16);
        // After removal the source and neighbours should be dark.
        assert_eq!(light.get(16, 16, 16).block_light(), 0);
        assert_eq!(light.get(17, 16, 16).block_light(), 0);
    }

    #[test]
    fn test_sunlight_blocked_by_floor() {
        let reg = test_registry();
        let (mut light, mut voxels) = make_empty_chunk();
        // Place a stone floor at y=16.
        for x in 0..32u32 {
            for z in 0..32u32 {
                voxels.set(x as usize, 16, z as usize, VoxelTypeId(1));
            }
        }
        propagate_sunlight(&mut light, &voxels, &reg);
        // Above the floor: sunlight = 15.
        assert_eq!(light.get(16, 17, 16).sunlight(), 15);
        // Below the floor: sunlight = 0 (no horizontal source nearby).
        assert_eq!(light.get(16, 15, 16).sunlight(), 0);
    }
}
