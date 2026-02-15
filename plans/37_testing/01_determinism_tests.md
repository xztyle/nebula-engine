# Determinism Tests

## Problem

The Nebula Engine must produce bit-for-bit identical results across runs given the same inputs. This is a hard requirement for three reasons:

1. **Multiplayer correctness** — The server is authoritative, but clients run prediction. If the client's predicted physics or terrain generation diverges from the server's by even one bit, desynchronization artifacts appear (rubber-banding, ghost blocks, phantom collisions). Determinism eliminates an entire class of desync bugs.

2. **Reproducible debugging** — When a bug is reported with a seed and input sequence, developers must be able to replay the exact scenario locally. Non-deterministic engines make this impossible.

3. **Cross-platform multiplayer** — Players on Linux, Windows, and macOS connect to the same server. If terrain generation or physics produce different results on different platforms, a Linux player and a Windows player looking at the same chunk will see different geometry. The engine uses integer-based 128-bit coordinates and fixed-point arithmetic specifically to avoid floating-point non-determinism, but this guarantee must be verified continuously.

Without automated determinism tests, regressions creep in silently — a developer adds a `HashMap` iteration (non-deterministic order), uses `f64` where fixed-point was required, or calls a platform-specific intrinsic, and the engine quietly becomes non-deterministic.

## Solution

### Terrain generation determinism

The terrain generator takes a seed (`u64`) and a chunk coordinate (`i64, i64, i64` plus cubesphere face index `u8`) and produces a fixed-size voxel array. Determinism means: same seed + same coordinate = identical byte output, every time, on every platform.

The test generates 1000 chunks in two separate passes and compares the output byte-for-byte.

```rust
use nebula_terrain::ChunkGenerator;
use nebula_voxel::ChunkData;

fn generate_chunk_set(seed: u64, count: usize) -> Vec<Vec<u8>> {
    let generator = ChunkGenerator::new(seed);
    (0..count)
        .map(|i| {
            let x = (i as i64) * 7 - 500;
            let y = ((i as i64) * 13) % 256;
            let z = (i as i64) * 3 + 100;
            let face = (i % 6) as u8;
            let chunk = generator.generate(x, y, z, face);
            chunk.to_bytes()
        })
        .collect()
}
```

### Physics determinism

The physics engine uses a fixed timestep and deterministic math (no `f64::sin` from libm — only the engine's own fixed-point or software-float implementations). The test creates a known initial state (N bodies with set positions, velocities, and masses), steps the simulation for 1000 ticks, and records the final state. A second run from the same initial state must produce an identical final state.

```rust
use nebula_physics::{PhysicsWorld, RigidBody, FixedVec3};

fn run_physics_simulation(bodies: &[RigidBody], ticks: u32) -> Vec<FixedVec3> {
    let mut world = PhysicsWorld::new();
    for body in bodies {
        world.add_body(body.clone());
    }
    for _ in 0..ticks {
        world.step();
    }
    world.bodies().iter().map(|b| b.position()).collect()
}
```

### Full simulation determinism

A higher-level test that exercises the ECS, terrain generation, physics, and entity spawning together. The test creates a `SimulationHarness` that wraps the full engine loop (minus rendering and IO), feeds it a scripted input sequence, runs for 600 ticks, and serializes the entire world state. Two runs must produce identical serialized output.

```rust
use nebula_engine::SimulationHarness;
use nebula_net::messages::serialize_message;

fn run_full_simulation(seed: u64, input_script: &[InputEvent], ticks: u32) -> Vec<u8> {
    let mut harness = SimulationHarness::new(seed);
    for event in input_script {
        harness.enqueue_input(event.clone());
    }
    for _ in 0..ticks {
        harness.tick();
    }
    harness.snapshot_world_state()
}
```

### Cross-platform determinism via CI

Each platform in the CI matrix (Linux, Windows, macOS) runs the terrain and physics determinism tests and uploads the serialized output as a CI artifact. A final CI job downloads all three artifacts and compares them byte-for-byte. If any platform diverges, the job fails.

```yaml
# In the CI workflow (story 06_cross_platform_ci)
- name: Run determinism tests and capture output
  run: cargo test --package nebula-testing -- --test-threads=1 determinism
  env:
    NEBULA_DETERMINISM_OUTPUT_DIR: ${{ runner.temp }}/determinism

- name: Upload determinism artifacts
  uses: actions/upload-artifact@v4
  with:
    name: determinism-${{ matrix.os }}
    path: ${{ runner.temp }}/determinism/
```

### Network state determinism

The test starts a server and two clients with identical input scripts. After processing the same message sequence, both clients' local game states must be identical. This validates that the networking layer does not introduce ordering-dependent non-determinism.

```rust
fn verify_network_determinism(message_sequence: &[Message]) -> bool {
    let state_a = replay_message_sequence(message_sequence);
    let state_b = replay_message_sequence(message_sequence);
    state_a == state_b
}
```

## Outcome

A `determinism_tests.rs` integration test file in `crates/nebula_testing/tests/` containing terrain, physics, simulation, network, and cross-platform determinism tests. A CI step that compares determinism artifacts across platforms. Any non-determinism in terrain generation, physics, entity simulation, or network state processing is caught automatically before code reaches `main`. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

The demo replays a recorded input sequence and compares the final world state hash against a known-good value. Any non-determinism between runs is caught and reported.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `serde` | `1.0` (features: `derive`) | Serialize world state snapshots for byte-level comparison |
| `postcard` | `1.1` (features: `alloc`) | Compact deterministic binary serialization of snapshots |
| `tokio` | `1.49` (features: `rt-multi-thread`, `macros`, `net`) | Async test runtime for network determinism tests |
| `tracing` | `0.1` | Structured logging for determinism test diagnostics |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Generate 1000 chunks with the same seed twice.
    /// Every chunk must be bitwise identical across both runs.
    #[test]
    fn test_terrain_generation_determinism_1000_chunks() {
        let seed = 0xDEAD_BEEF_CAFE_BABE_u64;
        let run_a = generate_chunk_set(seed, 1000);
        let run_b = generate_chunk_set(seed, 1000);
        assert_eq!(run_a.len(), 1000);
        assert_eq!(run_b.len(), 1000);
        for (i, (a, b)) in run_a.iter().zip(run_b.iter()).enumerate() {
            assert_eq!(
                a, b,
                "Chunk {i} diverged between runs: {} bytes vs {} bytes",
                a.len(),
                b.len()
            );
        }
    }

    /// Simulate 1000 physics ticks with identical initial conditions twice.
    /// Final positions of all bodies must be identical.
    #[test]
    fn test_physics_determinism_1000_ticks() {
        let bodies = create_test_bodies(50);
        let positions_a = run_physics_simulation(&bodies, 1000);
        let positions_b = run_physics_simulation(&bodies, 1000);
        assert_eq!(positions_a.len(), positions_b.len());
        for (i, (a, b)) in positions_a.iter().zip(positions_b.iter()).enumerate() {
            assert_eq!(
                a, b,
                "Body {i} position diverged: {a:?} vs {b:?}"
            );
        }
    }

    /// Full engine simulation: spawn 20 entities, run 600 ticks, serialize
    /// world state. Two runs must produce identical output.
    #[test]
    fn test_full_simulation_determinism_600_ticks() {
        let seed = 42;
        let inputs = create_scripted_input_sequence(20);
        let snapshot_a = run_full_simulation(seed, &inputs, 600);
        let snapshot_b = run_full_simulation(seed, &inputs, 600);
        assert_eq!(
            snapshot_a, snapshot_b,
            "World state diverged after 600 ticks: {} bytes vs {} bytes",
            snapshot_a.len(),
            snapshot_b.len()
        );
    }

    /// Replay the same network message sequence twice and verify the
    /// resulting game state is identical.
    #[tokio::test]
    async fn test_network_state_determinism() {
        let messages = create_test_message_sequence(100);
        let state_a = replay_message_sequence(&messages);
        let state_b = replay_message_sequence(&messages);
        assert_eq!(
            state_a, state_b,
            "Network state diverged after replaying the same message sequence"
        );
    }

    /// Verify that different seeds produce different terrain output —
    /// this is not a determinism test per se, but validates that the seed
    /// actually affects output (guards against the degenerate case where
    /// the generator ignores the seed and is trivially "deterministic").
    #[test]
    fn test_different_seeds_produce_different_terrain() {
        let chunks_a = generate_chunk_set(1, 10);
        let chunks_b = generate_chunk_set(2, 10);
        let any_different = chunks_a.iter().zip(chunks_b.iter()).any(|(a, b)| a != b);
        assert!(
            any_different,
            "Different seeds should produce different terrain data"
        );
    }

    /// Cross-platform comparison helper: serialize determinism output to a file.
    /// In CI, each platform writes its output and a final job compares them.
    #[test]
    fn test_determinism_output_can_be_serialized_for_cross_platform_comparison() {
        let seed = 0xCAFE_u64;
        let chunks = generate_chunk_set(seed, 10);
        let serialized = postcard::to_allocvec(&chunks).unwrap();
        assert!(!serialized.is_empty());
        let deserialized: Vec<Vec<u8>> = postcard::from_bytes(&serialized).unwrap();
        assert_eq!(chunks, deserialized);
    }
}
```
