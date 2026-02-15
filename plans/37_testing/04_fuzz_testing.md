# Fuzz Testing

## Problem

The Nebula Engine accepts untrusted input from multiple sources:

1. **Network messages** — Any TCP client can send arbitrary bytes to the server. The server deserializes these bytes with postcard 1.1 into `Message` variants. If the deserializer panics, overflows a buffer, or enters an infinite loop on crafted input, it becomes a denial-of-service vector or worse.

2. **Chunk data** — Chunk voxel data received over the network could be corrupted, truncated, or maliciously crafted. The deserialization and decompression path must handle every possible byte sequence without panicking.

3. **Configuration files** — The engine loads RON configuration files at startup. A malformed config file (from a modder, a corrupted save, or a crafted file) must not crash the engine.

4. **Voxel coordinates** — The 128-bit coordinate system has an enormous range. Edge cases (values near `i128::MIN`, `i128::MAX`, coordinates at cubesphere face boundaries, coordinates at zero) must be handled correctly. An integer overflow in coordinate math could cause terrain corruption or crashes.

5. **Script compilation** — The scripting engine (Rhai) compiles user-provided scripts. Malicious or malformed scripts must not cause the host engine to panic or hang.

Unit tests cover known edge cases, but fuzz testing explores the vast unknown input space. Fuzz testing has historically found bugs that no human would think to test: specific byte sequences that trigger integer overflow in varint decoding, chunk sizes that cause allocation failures, coordinate values that overflow during face-local conversion.

## Solution

### Fuzz target: network message deserialization

This is the highest-priority target because it faces untrusted network input directly. The fuzzer feeds arbitrary byte slices to `deserialize_message` and verifies it never panics.

```rust
// fuzz/fuzz_targets/fuzz_message_deserialize.rs
#![no_main]
use libfuzzer_sys::fuzz_target;
use nebula_net::messages::deserialize_message;

fuzz_target!(|data: &[u8]| {
    // deserialize_message must return Ok or Err — never panic.
    let _ = deserialize_message(data);
});
```

For deeper coverage, a structured fuzzer variant uses `Arbitrary` to generate valid-ish `Message` structs, serializes them, then mutates the serialized bytes before deserializing:

```rust
// fuzz/fuzz_targets/fuzz_message_roundtrip.rs
#![no_main]
use libfuzzer_sys::fuzz_target;
use arbitrary::Arbitrary;
use nebula_net::messages::{Message, serialize_message, deserialize_message};

#[derive(Arbitrary, Debug)]
struct FuzzInput {
    msg: Message,
    corruption_offset: usize,
    corruption_byte: u8,
}

fuzz_target!(|input: FuzzInput| {
    if let Ok(mut bytes) = serialize_message(&input.msg) {
        // Corrupt one byte
        if !bytes.is_empty() {
            let offset = input.corruption_offset % bytes.len();
            bytes[offset] = input.corruption_byte;
        }
        // Must not panic regardless of corruption
        let _ = deserialize_message(&bytes);
    }
});
```

### Fuzz target: chunk deserialization

```rust
// fuzz/fuzz_targets/fuzz_chunk_deserialize.rs
#![no_main]
use libfuzzer_sys::fuzz_target;
use nebula_voxel::ChunkData;

fuzz_target!(|data: &[u8]| {
    // Attempt to deserialize arbitrary bytes as chunk data.
    // The decompression and deserialization path must not panic.
    let _ = ChunkData::from_bytes(data);
});
```

### Fuzz target: RON config parsing

```rust
// fuzz/fuzz_targets/fuzz_config_parse.rs
#![no_main]
use libfuzzer_sys::fuzz_target;
use nebula_engine::config::EngineConfig;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        // Must return Ok or Err, never panic.
        let _ = ron::from_str::<EngineConfig>(text);
    }
});
```

### Fuzz target: voxel coordinate validation

```rust
// fuzz/fuzz_targets/fuzz_coordinate_validation.rs
#![no_main]
use libfuzzer_sys::fuzz_target;
use nebula_coords::{WorldCoord, CubesphereCoord};

#[derive(arbitrary::Arbitrary, Debug)]
struct CoordInput {
    x: i128,
    y: i128,
    z: i128,
    face: u8,
}

fuzz_target!(|input: CoordInput| {
    // Construction and conversion must not panic for any input values.
    let world = WorldCoord::new(input.x, input.y, input.z);
    let _ = world.to_local();
    let _ = world.to_cubesphere_face();

    if input.face < 6 {
        let cs = CubesphereCoord::new(input.x, input.y, input.z, input.face);
        let _ = cs.to_world();
    }
});
```

### Fuzz target: script compilation

```rust
// fuzz/fuzz_targets/fuzz_script_compile.rs
#![no_main]
use libfuzzer_sys::fuzz_target;
use rhai::Engine;

fuzz_target!(|data: &[u8]| {
    if let Ok(script) = std::str::from_utf8(data) {
        let engine = Engine::new();
        // Compilation must not panic. Evaluation is not tested here
        // because it could loop — only compilation is fuzzed.
        let _ = engine.compile(script);
    }
});
```

### Cargo-fuzz configuration

The `fuzz/Cargo.toml` sets up the fuzz workspace:

```toml
[package]
name = "nebula-fuzz"
version = "0.0.0"
edition = "2024"
publish = false

[package.metadata]
cargo-fuzz = true

[dependencies]
libfuzzer-sys = "0.4"
arbitrary = { version = "1.4", features = ["derive"] }
nebula_net = { path = "../crates/nebula_net" }
nebula_voxel = { path = "../crates/nebula_voxel" }
nebula_coords = { path = "../crates/nebula_coords" }
nebula_engine = { path = "../crates/nebula_engine" }
rhai = "1.23"
ron = "0.12"
serde = { version = "1.0", features = ["derive"] }
postcard = { version = "1.1", features = ["alloc"] }

[[bin]]
name = "fuzz_message_deserialize"
path = "fuzz_targets/fuzz_message_deserialize.rs"
doc = false

[[bin]]
name = "fuzz_message_roundtrip"
path = "fuzz_targets/fuzz_message_roundtrip.rs"
doc = false

[[bin]]
name = "fuzz_chunk_deserialize"
path = "fuzz_targets/fuzz_chunk_deserialize.rs"
doc = false

[[bin]]
name = "fuzz_config_parse"
path = "fuzz_targets/fuzz_config_parse.rs"
doc = false

[[bin]]
name = "fuzz_coordinate_validation"
path = "fuzz_targets/fuzz_coordinate_validation.rs"
doc = false

[[bin]]
name = "fuzz_script_compile"
path = "fuzz_targets/fuzz_script_compile.rs"
doc = false
```

### CI integration

Each fuzz target runs for 5 minutes in CI. This is enough to find shallow bugs but keeps CI runtime reasonable. A separate nightly job runs for 1 hour per target for deeper exploration.

```yaml
- name: Run fuzz targets (5 min each)
  run: |
    cargo install cargo-fuzz || true
    for target in fuzz_message_deserialize fuzz_message_roundtrip fuzz_chunk_deserialize fuzz_config_parse fuzz_coordinate_validation fuzz_script_compile; do
      echo "Fuzzing $target for 300 seconds..."
      cargo fuzz run "$target" -- -max_total_time=300 -max_len=4096
    done
```

### Corpus management

Initial seed inputs (valid messages, valid chunks, valid configs) are stored in `fuzz/corpus/<target>/` so the fuzzer starts from realistic inputs and mutates outward. Crash-triggering inputs are saved in `fuzz/artifacts/<target>/` and converted into regression tests.

## Outcome

A `fuzz/` directory at the repository root with 6 fuzz targets covering network message deserialization, message roundtrip corruption, chunk deserialization, RON config parsing, coordinate validation, and script compilation. Each target is integrated into CI with a 5-minute per-target time budget. Initial corpus seeds are provided for each target. Any crash-triggering input is automatically saved as an artifact. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Random byte sequences are fed to the message parser, voxel API, and scene loader. The demo must not crash, panic, or corrupt state. Any crash is filed as a bug.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `libfuzzer-sys` | `0.4` | LibFuzzer integration for Rust fuzz targets |
| `arbitrary` | `1.4` (features: `derive`) | Structured fuzzing — generate typed inputs from raw bytes |
| `postcard` | `1.1` (features: `alloc`) | Serialization under test in message fuzz targets |
| `serde` | `1.0` (features: `derive`) | Derive traits for fuzz input types |
| `ron` | `0.9` | RON config parser under test |
| `rhai` | `1.21` | Script engine under test for compilation fuzzing |
| `cargo-fuzz` | latest | CLI tool for managing and running fuzz targets |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that deserialize_message does not panic on empty input.
    #[test]
    fn test_message_deserializer_empty_input_no_panic() {
        let result = deserialize_message(&[]);
        assert!(result.is_err());
    }

    /// Verify that deserialize_message does not panic on random bytes.
    /// This is the deterministic equivalent of the fuzz target.
    #[test]
    fn test_message_deserializer_random_bytes_no_panic() {
        let test_inputs: Vec<Vec<u8>> = vec![
            vec![0xFF; 1],
            vec![0xFF; 100],
            vec![0x00; 1],
            vec![0x00; 1000],
            vec![PROTOCOL_VERSION, 0xFF, 0xFF, 0xFF, 0xFF],
            vec![PROTOCOL_VERSION],
            (0..256).map(|i| i as u8).collect(),
            vec![PROTOCOL_VERSION, 0, 0, 0, 0, 0, 0, 0],
        ];
        for input in &test_inputs {
            // Must not panic — Ok or Err are both acceptable.
            let _ = deserialize_message(input);
        }
    }

    /// Verify that ChunkData::from_bytes does not panic on corrupted data.
    #[test]
    fn test_chunk_deserializer_handles_corruption() {
        let corrupt_inputs: Vec<Vec<u8>> = vec![
            vec![],
            vec![0xFF; 10],
            vec![0x00; 32768],
            vec![0x01, 0x02, 0x03],
        ];
        for input in &corrupt_inputs {
            let _ = ChunkData::from_bytes(input);
        }
    }

    /// Verify that the RON config parser does not panic on invalid input.
    #[test]
    fn test_config_parser_handles_invalid_ron() {
        let invalid_inputs = vec![
            "",
            "not valid ron at all",
            "(((",
            "{ broken: }",
            &"x".repeat(100_000),
            "\0\0\0",
        ];
        for input in &invalid_inputs {
            let _ = ron::from_str::<EngineConfig>(input);
        }
    }

    /// Verify that coordinate validation handles extreme i128 values.
    #[test]
    fn test_coordinate_validation_handles_extremes() {
        let extreme_values = [i128::MIN, i128::MAX, 0, 1, -1, i128::MIN + 1, i128::MAX - 1];
        for &x in &extreme_values {
            for &y in &extreme_values {
                let coord = WorldCoord::new(x, y, 0);
                // Must not panic.
                let _ = coord.to_local();
                let _ = coord.to_cubesphere_face();
            }
        }
    }

    /// Verify that the Rhai engine does not panic when compiling invalid scripts.
    #[test]
    fn test_script_compilation_handles_invalid_scripts() {
        let engine = rhai::Engine::new();
        let invalid_scripts = vec![
            "",
            "fn(",
            "{{{{{{",
            "let x = ;",
            &"a + ".repeat(10_000),
            "\0\0\0",
        ];
        for script in &invalid_scripts {
            let _ = engine.compile(script);
        }
    }

    /// Verify that fuzz corpus seed files exist for each target.
    #[test]
    fn test_fuzz_corpus_seeds_exist() {
        let targets = [
            "fuzz_message_deserialize",
            "fuzz_message_roundtrip",
            "fuzz_chunk_deserialize",
            "fuzz_config_parse",
            "fuzz_coordinate_validation",
            "fuzz_script_compile",
        ];
        for target in &targets {
            let corpus_dir = format!("fuzz/corpus/{target}");
            assert!(
                std::path::Path::new(&corpus_dir).exists(),
                "Corpus directory should exist for fuzz target: {target}"
            );
        }
    }
}
```
