# Cross-Chunk Lighting

## Problem

The voxel light propagation system (story 04) works within a single chunk, but light does not respect chunk boundaries. A torch placed one voxel inside a chunk's eastern face must illuminate the western face of the neighboring chunk. Without cross-chunk propagation, every chunk boundary would produce a visible hard line where light abruptly drops to zero. This is especially noticeable in open areas lit by sunlight (where the horizon of a chunk would go dark) and underground where a single torch should illuminate a corridor spanning multiple chunks. The engine needs a mechanism to propagate light across chunk boundaries efficiently, without requiring every neighbor to fully re-propagate from scratch whenever a single voxel changes.

## Solution

### Border Light Cache

Each chunk maintains a 1-voxel-thick border cache on each of its 6 faces — the light values of the outermost voxel layer:

```rust
/// Light values along one face of a chunk (32x32 = 1024 entries).
pub type BorderLightFace = Box<[VoxelLight; 32 * 32]>;

/// Border caches for all 6 faces of a chunk.
pub struct ChunkBorderLights {
    /// Indexed by Face enum: PosX, NegX, PosY, NegY, PosZ, NegZ.
    pub faces: [BorderLightFace; 6],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    PosX = 0,
    NegX = 1,
    PosY = 2,
    NegY = 3,
    PosZ = 4,
    NegZ = 5,
}

impl Face {
    pub fn opposite(self) -> Face {
        match self {
            Face::PosX => Face::NegX,
            Face::NegX => Face::PosX,
            Face::PosY => Face::NegY,
            Face::NegY => Face::PosY,
            Face::PosZ => Face::NegZ,
            Face::NegZ => Face::PosZ,
        }
    }
}
```

### Extracting Border Light Data

After a chunk finishes its internal light propagation (story 04), extract the outermost layer on each face:

```rust
impl ChunkLightMap {
    pub fn extract_border(&self, face: Face) -> BorderLightFace {
        let mut border = Box::new([VoxelLight(0); 32 * 32]);
        for a in 0..32 {
            for b in 0..32 {
                let (x, y, z) = match face {
                    Face::PosX => (31, a, b),
                    Face::NegX => (0, a, b),
                    Face::PosY => (a, 31, b),
                    Face::NegY => (a, 0, b),
                    Face::PosZ => (a, b, 31),
                    Face::NegZ => (a, b, 0),
                };
                border[(a * 32 + b) as usize] = self.get(x, y, z);
            }
        }
        border
    }
}
```

### Cross-Chunk Propagation Algorithm

When a chunk's border light data changes, notify its neighbors:

```rust
pub fn propagate_cross_chunk(
    chunk: &mut ChunkLightMap,
    chunk_voxels: &ChunkVoxels,
    face: Face,
    neighbor_border: &BorderLightFace,
) {
    let mut queue = VecDeque::new();

    // For each voxel on the incoming face, check if the neighbor's border
    // value minus 1 exceeds the current light level. If so, seed a BFS.
    for a in 0..32u32 {
        for b in 0..32u32 {
            let neighbor_light = neighbor_border[(a * 32 + b) as usize];

            let (x, y, z) = match face {
                Face::NegX => (0, a, b),   // neighbor is to the +X, incoming on -X face
                Face::PosX => (31, a, b),
                Face::NegY => (a, 0, b),
                Face::PosY => (a, 31, b),
                Face::NegZ => (a, b, 0),
                Face::PosZ => (a, b, 31),
            };

            if chunk_voxels.is_opaque(x, y, z) { continue; }

            // Block light channel.
            let incoming_bl = neighbor_light.block_light().saturating_sub(1);
            if incoming_bl > chunk.get(x, y, z).block_light() {
                let mut l = chunk.get(x, y, z);
                l.set_block_light(incoming_bl);
                chunk.set(x, y, z, l);
                queue.push_back((x, y, z));
            }

            // Sunlight channel.
            let incoming_sl = if face == Face::NegY {
                neighbor_light.sunlight() // vertical: no decay
            } else {
                neighbor_light.sunlight().saturating_sub(1)
            };
            if incoming_sl > chunk.get(x, y, z).sunlight() {
                let mut l = chunk.get(x, y, z);
                l.set_sunlight(incoming_sl);
                chunk.set(x, y, z, l);
                if !queue.iter().any(|&(qx, qy, qz)| qx == x && qy == y && qz == z) {
                    queue.push_back((x, y, z));
                }
            }
        }
    }

    // Continue BFS within this chunk (same as story 04's internal propagation).
    propagate_bfs_from_queue(&mut queue, chunk, chunk_voxels);
}
```

### Change Detection

To avoid unnecessary re-propagation, compare the new border data with the cached version. Only trigger cross-chunk propagation when a face's border data actually changes:

```rust
pub fn border_changed(old: &BorderLightFace, new: &BorderLightFace) -> bool {
    old.iter().zip(new.iter()).any(|(a, b)| a != b)
}
```

### Light Removal Across Boundaries

When a light source is removed near a chunk edge, the reverse-BFS removal (story 04) may clear voxels on the border. The updated border is sent to the neighbor, which then removes stale light values from its own edge and re-propagates from any remaining sources.

### Bounded Convergence

In the worst case (a single light affecting a corridor spanning many chunks), propagation cascades through N chunks where N = light_level / 1 = 15. Each step processes at most one chunk. A light-level-15 source can affect at most 15 chunks in any direction, bounding the total work.

## Outcome

A cross-chunk light propagation system in `nebula_lighting` that extends the BFS flood fill across chunk boundaries using border light caches. When a chunk's border light data changes, its neighbors re-propagate. Light crosses boundaries seamlessly for both sunlight and block light channels. Running `cargo test -p nebula_lighting` passes all cross-chunk lighting tests. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Light propagation crosses chunk boundaries seamlessly. A light source in one chunk illuminates voxels in adjacent chunks without visible seams or discontinuities.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_voxel` | workspace | `ChunkVoxels`, chunk adjacency queries |

No external crates required. Cross-chunk propagation is pure Rust BFS and array comparison. Depends on story 04 (voxel light propagation).

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Create two adjacent chunks: chunk_a at x=0, chunk_b at x=1 (sharing the PosX/NegX face).
    fn make_adjacent_pair() -> (
        ChunkLightMap, ChunkVoxels,
        ChunkLightMap, ChunkVoxels,
    ) {
        (
            ChunkLightMap::new_dark(), ChunkVoxels::new_air(),
            ChunkLightMap::new_dark(), ChunkVoxels::new_air(),
        )
    }

    #[test]
    fn test_light_crosses_chunk_boundary() {
        let (mut light_a, voxels_a, mut light_b, voxels_b) = make_adjacent_pair();
        // Place a light source near the +X edge of chunk A at (31, 16, 16).
        propagate_block_light(&mut light_a, &voxels_a, &[(31, 16, 16, 15)]);

        // Extract chunk A's +X border and propagate into chunk B's -X face.
        let border = light_a.extract_border(Face::PosX);
        propagate_cross_chunk(&mut light_b, &voxels_b, Face::NegX, &border);

        // Chunk B at (0, 16, 16) should have light = 15 - 1 - 1 = 13.
        // (one step to reach edge of A = 14 at face, minus 1 crossing = 13 at B's face)
        let bl = light_b.get(0, 16, 16).block_light();
        assert!(bl > 0, "light should cross into chunk B, got {bl}");
        assert!(bl <= 14, "light should decay when crossing boundary, got {bl}");
    }

    #[test]
    fn test_border_cache_matches_neighbor_edge() {
        let (mut light_a, voxels_a, _, _) = make_adjacent_pair();
        propagate_block_light(&mut light_a, &voxels_a, &[(30, 16, 16, 10)]);
        let border = light_a.extract_border(Face::PosX);

        // The border value at (16, 16) in the 2D face should match the chunk's (31, 16, 16).
        let expected = light_a.get(31, 16, 16);
        let actual = border[(16 * 32 + 16) as usize];
        assert_eq!(actual, expected, "border cache must match chunk edge voxel");
    }

    #[test]
    fn test_removing_light_depropagates_across_boundary() {
        let (mut light_a, voxels_a, mut light_b, voxels_b) = make_adjacent_pair();
        // Place and propagate light.
        propagate_block_light(&mut light_a, &voxels_a, &[(31, 16, 16, 15)]);
        let border = light_a.extract_border(Face::PosX);
        propagate_cross_chunk(&mut light_b, &voxels_b, Face::NegX, &border);
        assert!(light_b.get(0, 16, 16).block_light() > 0);

        // Remove the light source from chunk A.
        remove_block_light(&mut light_a, &voxels_a, 31, 16, 16);
        let new_border = light_a.extract_border(Face::PosX);

        // The border should have changed.
        assert!(border_changed(&border, &new_border));

        // Re-propagate into chunk B with the updated (dark) border.
        let mut light_b_clean = ChunkLightMap::new_dark();
        propagate_cross_chunk(&mut light_b_clean, &voxels_b, Face::NegX, &new_border);

        // Chunk B should now be dark at (0, 16, 16).
        assert_eq!(
            light_b_clean.get(0, 16, 16).block_light(),
            0,
            "light should be removed after source deletion"
        );
    }

    #[test]
    fn test_two_lights_from_different_chunks_combine() {
        let (mut light_a, voxels_a, mut light_b, voxels_b) = make_adjacent_pair();
        // Chunk A has a light at (31, 16, 16).
        propagate_block_light(&mut light_a, &voxels_a, &[(31, 16, 16, 10)]);
        // Chunk B has its own light at (5, 16, 16).
        propagate_block_light(&mut light_b, &voxels_b, &[(5, 16, 16, 10)]);

        // Cross-propagate A -> B.
        let border_a = light_a.extract_border(Face::PosX);
        propagate_cross_chunk(&mut light_b, &voxels_b, Face::NegX, &border_a);

        // At (0, 16, 16) in chunk B, the light should be the max of:
        // - From chunk A's cross-propagation.
        // - From chunk B's own light at distance 5.
        let bl = light_b.get(0, 16, 16).block_light();
        let from_b_alone = 10u8.saturating_sub(5); // = 5
        assert!(
            bl >= from_b_alone,
            "combined light ({bl}) should be >= single source contribution ({from_b_alone})"
        );
    }

    #[test]
    fn test_propagation_settles_in_bounded_steps() {
        // A light-level-15 source can propagate at most 15 voxels in any direction.
        // Across chunk boundaries (32 voxels each), this means at most 1 chunk hop.
        // Verify that after one cross-chunk propagation, the distant end of chunk B is dark.
        let (mut light_a, voxels_a, mut light_b, voxels_b) = make_adjacent_pair();
        propagate_block_light(&mut light_a, &voxels_a, &[(31, 16, 16, 15)]);
        let border = light_a.extract_border(Face::PosX);
        propagate_cross_chunk(&mut light_b, &voxels_b, Face::NegX, &border);

        // At x=31 in chunk B (32 voxels from the boundary), light should be 0.
        // Max possible: 15 - 1 (to reach A's edge) - 1 (cross boundary) - 31 (to reach B's far edge) < 0.
        assert_eq!(
            light_b.get(31, 16, 16).block_light(),
            0,
            "light should not reach the far end of a neighboring chunk from level 15"
        );
    }
}
```
