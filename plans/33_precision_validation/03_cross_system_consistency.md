# Cross-System Consistency

## Problem

The Nebula Engine passes position data through multiple subsystems -- rendering, physics, voxel lookup, and networking -- each of which converts `WorldPosition` into its own internal representation. The rendering system converts to camera-relative `LocalPosition` (f32). The physics engine (Rapier) operates in its own f32 local space. The voxel system maps to integer chunk + block indices. The networking layer serializes positions as raw bytes for transmission. If any of these conversions disagree about the final position, the player sees desynchronized behavior: a block that renders at one location but has collision at another, or a remote player whose position jitters between two voxels. This story creates a "golden test" suite that feeds known `WorldPosition` values through every subsystem and asserts that all systems produce the same expected output.

## Solution

Create a cross-system consistency test module (file: `tests/cross_system_consistency.rs` at the workspace root, or as an integration test in a dedicated `nebula_integration_tests` crate) that imports types from `nebula_math`, `nebula_coords`, `nebula_physics`, `nebula_voxel`, and `nebula_net`.

### Golden test positions

Define a set of canonical test positions that cover the most important scenarios:

```rust
struct GoldenTestCase {
    /// Human-readable description
    name: &'static str,
    /// The canonical world position
    world_pos: WorldPosition,
    /// Camera / origin for this test
    camera_origin: WorldPosition,
    /// Expected local position after world-to-local conversion (f32)
    expected_local: LocalPosition,
    /// Expected chunk address for voxel lookup
    expected_chunk: ChunkAddress,
    /// Expected block index within the chunk
    expected_block: BlockIndex,
    /// Expected sector coordinate
    expected_sector: SectorCoord,
}

const GOLDEN_TESTS: &[GoldenTestCase] = &[
    GoldenTestCase {
        name: "origin_standing_on_surface",
        world_pos: WorldPosition::new(0, 6_371_000_000, 0),
        camera_origin: WorldPosition::new(0, 6_371_000_000, 0),
        expected_local: LocalPosition::new(0.0, 0.0, 0.0),
        expected_chunk: ChunkAddress::new(CubeFace::Top, 0, 0, 0),
        expected_block: BlockIndex::new(0, 0, 0),
        expected_sector: SectorCoord {
            sector: SectorIndex { x: 0, y: 1, z: 0 },
            offset: SectorOffset { x: 0, y: 2_076_032_704, z: 0 },
        },
    },
    GoldenTestCase {
        name: "one_meter_east",
        world_pos: WorldPosition::new(1_000, 6_371_000_000, 0),
        camera_origin: WorldPosition::new(0, 6_371_000_000, 0),
        expected_local: LocalPosition::new(1000.0, 0.0, 0.0),
        expected_chunk: ChunkAddress::new(CubeFace::Top, 0, 0, 0),
        expected_block: BlockIndex::new(1, 0, 0),
        expected_sector: SectorCoord {
            sector: SectorIndex { x: 0, y: 1, z: 0 },
            offset: SectorOffset { x: 1_000, y: 2_076_032_704, z: 0 },
        },
    },
    GoldenTestCase {
        name: "five_km_away",
        world_pos: WorldPosition::new(5_000_000, 6_371_000_000, 0),
        camera_origin: WorldPosition::new(0, 6_371_000_000, 0),
        expected_local: LocalPosition::new(5_000_000.0, 0.0, 0.0),
        expected_chunk: ChunkAddress::new(CubeFace::Top, 5, 0, 0),
        expected_block: BlockIndex::new(0, 0, 0),
        expected_sector: SectorCoord {
            sector: SectorIndex { x: 0, y: 1, z: 0 },
            offset: SectorOffset { x: 5_000_000, y: 2_076_032_704, z: 0 },
        },
    },
    GoldenTestCase {
        name: "negative_quadrant",
        world_pos: WorldPosition::new(-500_000, 6_370_999_000, -250_000),
        camera_origin: WorldPosition::new(0, 6_371_000_000, 0),
        expected_local: LocalPosition::new(-500_000.0, -1_000.0, -250_000.0),
        expected_chunk: ChunkAddress::new(CubeFace::Top, -1, 0, -1),
        expected_block: BlockIndex::new(15, 0, 15),
        expected_sector: SectorCoord {
            sector: SectorIndex { x: -1, y: 1, z: -1 },
            offset: SectorOffset {
                x: ((-500_000_i128) & 0xFFFF_FFFF) as i32,
                y: 2_076_031_704,
                z: ((-250_000_i128) & 0xFFFF_FFFF) as i32,
            },
        },
    },
];
```

### Per-system verification

For each golden test case, run the position through each subsystem and compare:

#### Rendering system

```rust
fn verify_render_position(case: &GoldenTestCase) {
    let local = to_local(case.world_pos, case.camera_origin);
    assert!(
        local.approx_eq(case.expected_local, 0.5),
        "[{}] Render position mismatch: got {:?}, expected {:?}",
        case.name, local, case.expected_local,
    );
}
```

#### Physics system

```rust
fn verify_physics_position(case: &GoldenTestCase) {
    // Physics uses the same world-to-local conversion as rendering
    let physics_local = physics_world_to_local(case.world_pos, case.camera_origin);
    let render_local = to_local(case.world_pos, case.camera_origin);
    assert!(
        physics_local.approx_eq(render_local, 0.001),
        "[{}] Physics and render positions disagree: physics={:?}, render={:?}",
        case.name, physics_local, render_local,
    );
}
```

#### Voxel lookup

```rust
fn verify_voxel_lookup(case: &GoldenTestCase) {
    let (chunk, block) = voxel_address_from_world(case.world_pos);
    assert_eq!(
        chunk, case.expected_chunk,
        "[{}] Chunk address mismatch", case.name,
    );
    assert_eq!(
        block, case.expected_block,
        "[{}] Block index mismatch", case.name,
    );
}
```

#### Network serialization

```rust
fn verify_network_roundtrip(case: &GoldenTestCase) {
    let bytes = serialize_world_position(&case.world_pos);
    let deserialized = deserialize_world_position(&bytes);
    assert_eq!(
        deserialized, case.world_pos,
        "[{}] Network roundtrip changed position", case.name,
    );
}
```

### All-systems agreement

The final test asserts that for the same entity, all systems agree:

```rust
fn verify_all_systems_agree(case: &GoldenTestCase) {
    let render_local = to_local(case.world_pos, case.camera_origin);
    let physics_local = physics_world_to_local(case.world_pos, case.camera_origin);
    let (chunk, block) = voxel_address_from_world(case.world_pos);
    let net_roundtrip = {
        let bytes = serialize_world_position(&case.world_pos);
        deserialize_world_position(&bytes)
    };

    // Render and physics must agree within 0.5 mm
    assert!(render_local.approx_eq(physics_local, 0.5));
    // Voxel lookup must return the expected chunk and block
    assert_eq!(chunk, case.expected_chunk);
    assert_eq!(block, case.expected_block);
    // Network must preserve exact value
    assert_eq!(net_roundtrip, case.world_pos);
}
```

## Outcome

After this story is complete:

- A golden test suite defines canonical positions with expected outputs for every subsystem
- The rendering system's local position matches the expected f32 value for each test case
- The physics system's local position agrees with rendering to within 0.5 mm
- The voxel lookup returns the correct chunk address and block index for each test case
- Network serialization/deserialization preserves the exact `WorldPosition` bit-for-bit
- A single test function (`verify_all_systems_agree`) confirms that all four systems agree on the same entity's position
- Adding a new golden test case is as simple as appending to the `GOLDEN_TESTS` array
- Running `cargo test -p nebula_integration_tests -- cross_system_consistency` passes all tests

## Demo Integration

**Demo crate:** `nebula-demo`

The same world position is converted through physics, rendering, and networking subsystems. All agree on the result. The console shows cross-system validation passing.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_math` | workspace | `WorldPosition`, `LocalPosition`, `Vec3I128`, `to_local`, `to_world` |
| `nebula_coords` | workspace | `SectorCoord`, `SectorIndex`, `SectorOffset` |
| `nebula_voxel` | workspace | `ChunkAddress`, `BlockIndex`, `voxel_address_from_world` |
| `nebula_physics` | workspace | `physics_world_to_local` |
| `nebula_net` | workspace | `serialize_world_position`, `deserialize_world_position` |

Rust edition 2024. No external crates beyond workspace members.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_position_matches_expected() {
        for case in GOLDEN_TESTS {
            let local = to_local(case.world_pos, case.camera_origin);
            assert!(
                local.approx_eq(case.expected_local, 0.5),
                "[{}] Render: got {:?}, expected {:?}",
                case.name, local, case.expected_local,
            );
        }
    }

    #[test]
    fn test_physics_position_matches_expected() {
        for case in GOLDEN_TESTS {
            let physics_local = physics_world_to_local(case.world_pos, case.camera_origin);
            assert!(
                physics_local.approx_eq(case.expected_local, 0.5),
                "[{}] Physics: got {:?}, expected {:?}",
                case.name, physics_local, case.expected_local,
            );
        }
    }

    #[test]
    fn test_voxel_lookup_matches_expected() {
        for case in GOLDEN_TESTS {
            let (chunk, block) = voxel_address_from_world(case.world_pos);
            assert_eq!(
                chunk, case.expected_chunk,
                "[{}] Chunk address mismatch", case.name,
            );
            assert_eq!(
                block, case.expected_block,
                "[{}] Block index mismatch", case.name,
            );
        }
    }

    #[test]
    fn test_network_roundtrip_matches() {
        for case in GOLDEN_TESTS {
            let bytes = serialize_world_position(&case.world_pos);
            let deserialized = deserialize_world_position(&bytes);
            assert_eq!(
                deserialized, case.world_pos,
                "[{}] Network roundtrip mismatch", case.name,
            );
        }
    }

    #[test]
    fn test_all_systems_agree_on_same_entity() {
        for case in GOLDEN_TESTS {
            let render_local = to_local(case.world_pos, case.camera_origin);
            let physics_local = physics_world_to_local(case.world_pos, case.camera_origin);

            // Rendering and physics must agree
            assert!(
                render_local.approx_eq(physics_local, 0.5),
                "[{}] Render ({:?}) and physics ({:?}) disagree",
                case.name, render_local, physics_local,
            );

            // Voxel must be consistent
            let (chunk, _block) = voxel_address_from_world(case.world_pos);
            assert_eq!(chunk, case.expected_chunk, "[{}] Voxel disagrees", case.name);

            // Network must preserve exactly
            let bytes = serialize_world_position(&case.world_pos);
            let recovered = deserialize_world_position(&bytes);
            assert_eq!(recovered, case.world_pos, "[{}] Network disagrees", case.name);
        }
    }

    #[test]
    fn test_render_and_physics_use_same_origin() {
        // Verify that both systems use the same camera origin, not different origins
        let origin = WorldPosition::new(1_000_000_000, 2_000_000_000, 3_000_000_000);
        let pos = WorldPosition::new(1_000_001_000, 2_000_002_000, 3_000_003_000);
        let render = to_local(pos, origin);
        let physics = physics_world_to_local(pos, origin);
        assert!(
            render.approx_eq(physics, 0.001),
            "Render and physics must use identical origin-subtraction logic"
        );
    }
}
```
