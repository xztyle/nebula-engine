# Determinism Validation

## Problem

The Nebula Engine must be deterministic: given the same inputs, every system must produce bit-identical outputs. This is critical for three reasons. First, multiplayer networking uses a client-prediction model where each client simulates the same game state locally -- if simulation diverges even by one bit, clients desynchronize and the server must perform expensive corrections. Second, terrain generation from a seed must produce identical chunks on every client and server, or players see different worlds. Third, saved games and replays depend on deterministic replay of recorded inputs. Non-determinism can creep in through: unordered iteration of hash maps, floating-point fast-math optimizations (fused multiply-add, reassociation), thread scheduling, and platform-specific differences in math libraries. This story builds a test harness that detects any non-determinism.

## Solution

Create a determinism test module (file: `tests/determinism_validation.rs` at the workspace root) that runs operations twice with identical inputs and performs binary comparison of the outputs.

### Strategy

Each test follows the pattern:

1. Set up identical initial conditions (seed, entity state, input sequence).
2. Run the operation once, capture the full output state as bytes.
3. Reset to the same initial conditions.
4. Run the operation again, capture the output state as bytes.
5. Assert the two byte sequences are identical.

### Terrain generation determinism

```rust
/// Generate a chunk from a seed and return its voxel data as a byte array.
fn generate_chunk_bytes(seed: u64, chunk_addr: ChunkAddress) -> Vec<u8> {
    let generator = TerrainGenerator::new(seed);
    let chunk = generator.generate_chunk(chunk_addr);
    chunk.serialize_to_bytes()
}
```

Test: generate the same chunk twice from the same seed. The raw byte output must be identical.

### Physics simulation determinism

```rust
/// Run N physics ticks with a fixed timestep and return the serialized state.
fn run_physics_simulation(
    initial_state: &PhysicsSnapshot,
    inputs: &[PhysicsInput],
    ticks: u32,
) -> Vec<u8> {
    let mut world = PhysicsWorld::from_snapshot(initial_state);
    for tick in 0..ticks {
        if let Some(input) = inputs.get(tick as usize) {
            world.apply_input(input);
        }
        world.step(FIXED_TIMESTEP);
    }
    world.snapshot().serialize_to_bytes()
}
```

Test: run the same simulation twice. Assert byte-identical results.

### Float determinism constraints

The engine must **not** use compiler flags or intrinsics that break IEEE 754 determinism:

- No `-ffast-math` equivalent (`#[cfg(target_feature)]` for FMA must be explicit and consistent).
- No `f32::mul_add` unless explicitly opted in (hardware FMA can produce different results than separate mul + add).
- All `f32`/`f64` operations must use the same rounding mode (default: round-to-nearest-even).
- Hash map iteration order must not affect results. Where order matters, sort keys first or use `BTreeMap`.

```rust
/// Verify that f32 operations produce identical results without fast-math.
fn verify_f32_determinism() {
    let a: f32 = 1.0000001;
    let b: f32 = 0.9999999;
    let c: f32 = 1000000.0;

    // (a * c) + (b * c) might differ from a.mul_add(c, b * c) on some hardware
    let result1 = (a * c) + (b * c);
    let result2 = (a * c) + (b * c);
    assert_eq!(result1.to_bits(), result2.to_bits(), "f32 must be bit-deterministic");
}
```

### Entity iteration order

Systems that iterate over entities must produce the same result regardless of ECS internal ordering. The test inserts entities in different orders and verifies the simulation output is identical:

```rust
fn verify_iteration_order_independence() {
    // Insert entities A, B, C in order [A, B, C]
    let state_1 = run_simulation_with_entity_order(&[entity_a, entity_b, entity_c]);
    // Insert entities in order [C, A, B]
    let state_2 = run_simulation_with_entity_order(&[entity_c, entity_a, entity_b]);
    assert_eq!(state_1, state_2, "Simulation must not depend on entity insertion order");
}
```

### Cross-client determinism

Simulate two "clients" in the same process with the same initial state and input sequence. Compare their states after N ticks:

```rust
fn verify_two_clients_agree(seed: u64, inputs: &[InputFrame], ticks: u32) {
    let state_a = simulate_client(seed, inputs, ticks);
    let state_b = simulate_client(seed, inputs, ticks);
    assert_eq!(
        state_a, state_b,
        "Two clients with identical inputs must produce identical state"
    );
}
```

## Outcome

After this story is complete:

- Terrain generation is proven deterministic: same seed and chunk address always produce identical voxel data
- Physics simulation is proven deterministic: same initial state and inputs always produce identical final state
- f32 operations are verified to be bit-identical across repeated runs (no fast-math interference)
- Entity iteration order does not affect simulation results
- Two simulated clients with the same inputs produce identical game state
- Any future non-determinism regression is caught by the test suite
- Running `cargo test -p nebula_integration_tests -- determinism` passes all tests

## Demo Integration

**Demo crate:** `nebula-demo`

The demo runs 100 simulation ticks twice with identical input. The final world state matches bit-for-bit. The console shows `Determinism: PASS, 100/100 ticks identical`.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_math` | workspace | `WorldPosition`, `Vec3I128`, coordinate math |
| `nebula_terrain` | workspace | `TerrainGenerator`, chunk generation |
| `nebula_physics` | workspace | `PhysicsWorld`, simulation stepping |
| `nebula_ecs` | workspace | Entity management, system scheduling |
| `nebula_voxel` | workspace | `Chunk`, serialization |
| `nebula_net` | workspace | Snapshot serialization |

Rust edition 2024. No external crates beyond workspace members. The tests rely on the engine's own serialization to produce byte arrays for comparison.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terrain_generation_is_deterministic() {
        let seed: u64 = 12345;
        let addr = ChunkAddress::new(CubeFace::Top, 7, 3, -2);

        let bytes_a = generate_chunk_bytes(seed, addr);
        let bytes_b = generate_chunk_bytes(seed, addr);

        assert_eq!(
            bytes_a, bytes_b,
            "Terrain generation with same seed must produce identical bytes"
        );
    }

    #[test]
    fn test_terrain_different_seeds_differ() {
        let addr = ChunkAddress::new(CubeFace::Top, 0, 0, 0);

        let bytes_a = generate_chunk_bytes(111, addr);
        let bytes_b = generate_chunk_bytes(222, addr);

        assert_ne!(
            bytes_a, bytes_b,
            "Different seeds must produce different terrain (sanity check)"
        );
    }

    #[test]
    fn test_physics_simulation_is_deterministic() {
        let initial = PhysicsSnapshot::default();
        let inputs: Vec<PhysicsInput> = vec![
            PhysicsInput::ApplyForce(Vec3I128::new(1000, 0, 0)),
            PhysicsInput::ApplyForce(Vec3I128::new(0, 1000, 0)),
            PhysicsInput::ApplyForce(Vec3I128::new(0, 0, 1000)),
        ];

        let state_a = run_physics_simulation(&initial, &inputs, 300);
        let state_b = run_physics_simulation(&initial, &inputs, 300);

        assert_eq!(
            state_a, state_b,
            "Physics simulation with same inputs must produce identical state"
        );
    }

    #[test]
    fn test_two_clients_same_inputs_same_state() {
        let seed: u64 = 42;
        let inputs: Vec<InputFrame> = vec![
            InputFrame::MoveForward,
            InputFrame::MoveForward,
            InputFrame::Jump,
            InputFrame::MoveLeft,
        ];

        let state_a = simulate_client(seed, &inputs, 600);
        let state_b = simulate_client(seed, &inputs, 600);

        assert_eq!(
            state_a, state_b,
            "Two clients with identical inputs must produce identical state"
        );
    }

    #[test]
    fn test_f32_operations_are_deterministic() {
        // Run the same f32 computation 100 times and verify bit-identical results
        let mut results = Vec::new();
        for _ in 0..100 {
            let a: f32 = 1.0000001;
            let b: f32 = 0.9999999;
            let c: f32 = 1_000_000.0;
            let result = (a * c) + (b * c);
            results.push(result.to_bits());
        }
        let first = results[0];
        assert!(
            results.iter().all(|&r| r == first),
            "f32 operations must produce bit-identical results across 100 runs"
        );
    }

    #[test]
    fn test_order_of_operations_is_stable() {
        // Verify that entity processing order does not affect outcome
        let entities_order_a = vec![
            (0u64, WorldPosition::new(100, 200, 300)),
            (1u64, WorldPosition::new(400, 500, 600)),
            (2u64, WorldPosition::new(700, 800, 900)),
        ];
        let entities_order_b = vec![
            (2u64, WorldPosition::new(700, 800, 900)),
            (0u64, WorldPosition::new(100, 200, 300)),
            (1u64, WorldPosition::new(400, 500, 600)),
        ];

        let state_a = run_simulation_with_entity_order(&entities_order_a);
        let state_b = run_simulation_with_entity_order(&entities_order_b);

        assert_eq!(
            state_a, state_b,
            "Entity insertion order must not affect simulation result"
        );
    }

    #[test]
    fn test_i128_arithmetic_is_deterministic() {
        // i128 arithmetic is always deterministic (no hardware variance),
        // but verify as a baseline
        let a = WorldPosition::new(
            123_456_789_012_345_678,
            -987_654_321_098_765_432,
            42,
        );
        let b = WorldPosition::new(
            111_111_111_111_111_111,
            -222_222_222_222_222_222,
            333_333_333_333_333_333,
        );

        let result_1 = a - b;
        let result_2 = a - b;
        assert_eq!(result_1, result_2, "i128 subtraction must be deterministic");

        let dist_1 = distance_squared(a, b);
        let dist_2 = distance_squared(a, b);
        assert_eq!(dist_1, dist_2, "distance_squared must be deterministic");
    }
}
```
