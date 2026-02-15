# Validation Layers

## Problem

Logic errors in a game engine are often silent. A position with `NaN` coordinates does not crash immediately -- it propagates through physics, rendering, and networking, producing increasingly bizarre behavior until something finally fails catastrophically far from the original bug. A quaternion that is not normalized produces subtly incorrect rotations. A voxel coordinate outside the chunk bounds produces an out-of-bounds array access (a panic) or, worse, wraps around and silently corrupts adjacent data. A buffer uploaded to the GPU with the wrong size produces garbled rendering or a driver crash.

These bugs are difficult to diagnose because the symptom appears far from the cause. In graphics programming, this problem is solved by validation layers (Vulkan validation layers, D3D debug layer) that check every API call for correctness and report violations immediately. The same principle should be applied at the engine level: every critical input should be validated at the point of use, with a clear error message pointing directly at the bug.

However, validation must have zero runtime cost in release builds. Players should not pay for checks that only matter during development. Rust's `debug_assert!` macro provides exactly this: assertions that are compiled out in release mode but active in debug builds and during testing.

## Solution

### Validation Helper Functions

Create a `nebula_validation` module (or embed validation functions in each crate) with helper functions that perform common checks. Each function uses `debug_assert!` internally and provides a descriptive error message:

```rust
/// Validate that a floating-point value is finite (not NaN or infinity).
/// Panics in debug builds if the value is not finite.
#[inline(always)]
pub fn validate_finite(value: f32, name: &str) {
    debug_assert!(
        value.is_finite(),
        "Validation failed: {name} is not finite (value: {value})"
    );
}

/// Validate that a 3D vector has all finite components.
#[inline(always)]
pub fn validate_vec3_finite(v: [f32; 3], name: &str) {
    debug_assert!(
        v[0].is_finite() && v[1].is_finite() && v[2].is_finite(),
        "Validation failed: {name} has non-finite components: [{}, {}, {}]",
        v[0], v[1], v[2]
    );
}

/// Validate that a quaternion is normalized (length approximately 1.0).
/// Tolerance accounts for floating-point accumulation errors.
#[inline(always)]
pub fn validate_quaternion_normalized(q: [f32; 4], name: &str) {
    let len_sq = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
    debug_assert!(
        (len_sq - 1.0).abs() < 1e-4,
        "Validation failed: {name} is not normalized (length squared: {len_sq}, \
         expected ~1.0, components: [{}, {}, {}, {}])",
        q[0], q[1], q[2], q[3]
    );
}

/// Validate that chunk coordinates are within the valid world range.
#[inline(always)]
pub fn validate_chunk_coords(x: i32, y: i32, z: i32, max_coord: i32) {
    debug_assert!(
        x.abs() <= max_coord && y.abs() <= max_coord && z.abs() <= max_coord,
        "Validation failed: chunk coordinates ({x}, {y}, {z}) exceed \
         maximum range +/-{max_coord}"
    );
}

/// Validate that local voxel coordinates are within the chunk.
#[inline(always)]
pub fn validate_voxel_local_coords(x: usize, y: usize, z: usize, chunk_size: usize) {
    debug_assert!(
        x < chunk_size && y < chunk_size && z < chunk_size,
        "Validation failed: voxel local coordinates ({x}, {y}, {z}) \
         are out of bounds for chunk size {chunk_size}"
    );
}

/// Validate that a voxel type ID is registered in the palette/registry.
#[inline(always)]
pub fn validate_voxel_type_registered(type_id: u16, max_registered: u16) {
    debug_assert!(
        type_id < max_registered,
        "Validation failed: voxel type ID {type_id} is not registered \
         (max registered ID: {})",
        max_registered.saturating_sub(1)
    );
}

/// Validate that a buffer size matches the expected size.
#[inline(always)]
pub fn validate_buffer_size(actual: usize, expected: usize, name: &str) {
    debug_assert!(
        actual == expected,
        "Validation failed: buffer '{name}' has size {actual} bytes, \
         expected {expected} bytes"
    );
}

/// Validate that a slice length matches the expected count.
#[inline(always)]
pub fn validate_slice_length<T>(slice: &[T], expected: usize, name: &str) {
    debug_assert!(
        slice.len() == expected,
        "Validation failed: slice '{name}' has length {}, expected {expected}",
        slice.len()
    );
}

/// Validate that an index is within bounds.
#[inline(always)]
pub fn validate_index(index: usize, len: usize, name: &str) {
    debug_assert!(
        index < len,
        "Validation failed: index {index} out of bounds for {name} \
         with length {len}"
    );
}

/// Validate that a value is within a specified range (inclusive).
#[inline(always)]
pub fn validate_range(value: f32, min: f32, max: f32, name: &str) {
    debug_assert!(
        value >= min && value <= max,
        "Validation failed: {name} value {value} is outside \
         valid range [{min}, {max}]"
    );
}

/// Validate that a matrix is not degenerate (determinant is not near zero).
#[inline(always)]
pub fn validate_matrix_non_degenerate(determinant: f32, name: &str) {
    debug_assert!(
        determinant.abs() > 1e-8,
        "Validation failed: matrix '{name}' is degenerate \
         (determinant: {determinant})"
    );
}
```

### Integration Points

Validation calls are placed at critical boundaries throughout the engine. These are the points where invalid data would cause cascading failures:

#### Voxel System

```rust
pub fn set_voxel(&mut self, x: usize, y: usize, z: usize, voxel_type: u16) {
    validate_voxel_local_coords(x, y, z, CHUNK_SIZE);
    validate_voxel_type_registered(voxel_type, self.registry.type_count());

    self.data[x + y * CHUNK_SIZE + z * CHUNK_SIZE * CHUNK_SIZE] = voxel_type;
}

pub fn get_voxel(&self, x: usize, y: usize, z: usize) -> u16 {
    validate_voxel_local_coords(x, y, z, CHUNK_SIZE);

    self.data[x + y * CHUNK_SIZE + z * CHUNK_SIZE * CHUNK_SIZE]
}
```

#### Transform System

```rust
pub fn set_position(&mut self, pos: [f32; 3]) {
    validate_vec3_finite(pos, "position");
    self.position = pos;
}

pub fn set_rotation(&mut self, quat: [f32; 4]) {
    validate_quaternion_normalized(quat, "rotation");
    self.rotation = quat;
}

pub fn set_scale(&mut self, scale: [f32; 3]) {
    validate_vec3_finite(scale, "scale");
    debug_assert!(
        scale[0] > 0.0 && scale[1] > 0.0 && scale[2] > 0.0,
        "Validation failed: scale must be positive, got [{}, {}, {}]",
        scale[0], scale[1], scale[2]
    );
    self.scale = scale;
}
```

#### GPU Buffer Upload

```rust
pub fn upload_vertex_buffer(
    &self,
    device: &wgpu::Device,
    vertices: &[Vertex],
    expected_count: usize,
) -> wgpu::Buffer {
    validate_slice_length(vertices, expected_count, "vertex_buffer");

    let byte_size = std::mem::size_of_val(vertices);
    validate_buffer_size(byte_size, expected_count * std::mem::size_of::<Vertex>(), "vertex_buffer_bytes");

    // Validate no NaN in vertex positions
    for (i, vertex) in vertices.iter().enumerate() {
        debug_assert!(
            vertex.position[0].is_finite()
                && vertex.position[1].is_finite()
                && vertex.position[2].is_finite(),
            "Validation failed: vertex {i} has non-finite position: {:?}",
            vertex.position
        );
    }

    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("vertex_buffer"),
        contents: bytemuck::cast_slice(vertices),
        usage: wgpu::BufferUsages::VERTEX,
    })
}
```

#### Chunk Addressing

```rust
pub fn load_chunk(&mut self, pos: ChunkPos) {
    validate_chunk_coords(pos.x, pos.y, pos.z, MAX_CHUNK_COORD);
    // ... chunk loading logic
}
```

### Conditional Compilation

All validation functions use `#[inline(always)]` and `debug_assert!`, which means:

- **Debug builds** (`cargo build`, `cargo test`): Assertions are active. Invalid inputs trigger an immediate panic with a descriptive message pointing to the exact validation that failed.
- **Release builds** (`cargo build --release`): All `debug_assert!` calls are compiled out entirely. The validation functions become empty (no-ops) and are eliminated by the optimizer. There is literally zero runtime cost.

For cases where validation is wanted even in release builds (e.g., validating user-provided script input or network messages from untrusted sources), use regular `assert!` or return `Result`:

```rust
/// Validate untrusted input (active in all builds).
pub fn validate_network_message_size(size: usize, max: usize) -> Result<(), NetworkError> {
    if size > max {
        return Err(NetworkError::MessageTooLarge { size, max });
    }
    Ok(())
}
```

### Validation Configuration

A compile-time feature flag allows enabling extended validation in release builds for testing:

```toml
[features]
default = []
validation = []  # Enable debug_assert-like checks even in release
```

```rust
macro_rules! engine_assert {
    ($cond:expr, $($arg:tt)*) => {
        if cfg!(debug_assertions) || cfg!(feature = "validation") {
            assert!($cond, $($arg)*);
        }
    };
}
```

This allows CI to run release-mode tests with validation enabled (`cargo test --release --features validation`) to catch issues that only manifest under release optimizations.

## Outcome

Every critical input boundary in the engine has debug-build-only validation checks. Invalid positions (NaN, infinity), unnormalized quaternions, out-of-range chunk coordinates, unregistered voxel type IDs, and mismatched buffer sizes are caught immediately with descriptive error messages that point directly to the bug. Release builds pay zero runtime cost -- all `debug_assert!` calls are compiled out. A `validation` feature flag enables checks in release builds for CI testing. Untrusted input (network messages, user scripts) is validated with `Result`-returning functions in all builds.

## Demo Integration

**Demo crate:** `nebula-demo`

In debug builds, wgpu validation layers catch GPU API misuse (wrong bind group, mismatched vertex format) immediately with descriptive errors instead of silent corruption.

## Crates & Dependencies

No external crates are required. All validation uses standard library features:

- **`debug_assert!`** -- Standard library macro, compiled out in release builds.
- **`assert!`** -- Standard library macro, active in all builds (used for untrusted input validation).
- **`cfg!(debug_assertions)`** -- Compile-time check for debug vs. release mode.
- **`cfg!(feature = "validation")`** -- Compile-time feature flag for opt-in release validation.

## Unit Tests

- **`test_valid_input_passes_validation`** -- Call `validate_finite(1.0, "test")`, `validate_vec3_finite([1.0, 2.0, 3.0], "pos")`, `validate_quaternion_normalized([0.0, 0.0, 0.0, 1.0], "rot")`, `validate_voxel_local_coords(15, 15, 15, 32)`, `validate_chunk_coords(100, -50, 200, 1000)`, `validate_buffer_size(64, 64, "buf")`, and `validate_index(5, 10, "arr")`. Assert none of them panic (the test passes if it completes without panicking).

- **`test_nan_position_triggers_assert`** -- Call `validate_vec3_finite([f32::NAN, 0.0, 0.0], "position")` inside `#[should_panic(expected = "not finite")]`. Assert the test panics with the expected message. Repeat for infinity: `validate_finite(f32::INFINITY, "value")` should also panic.

- **`test_unnormalized_quaternion_triggers_assert`** -- Call `validate_quaternion_normalized([1.0, 1.0, 1.0, 1.0], "rotation")` inside `#[should_panic(expected = "not normalized")]`. The length squared is 4.0, far from 1.0, so this must trigger. Also test with `[0.0, 0.0, 0.0, 0.0]` (zero quaternion, length 0.0).

- **`test_out_of_bounds_voxel_coords_triggers_assert`** -- Call `validate_voxel_local_coords(32, 0, 0, 32)` inside `#[should_panic(expected = "out of bounds")]`. The coordinate 32 is not less than chunk_size 32, so this must trigger. Test with `(0, 0, 33, 32)` as well.

- **`test_release_build_skips_validation`** -- This is a compile-time guarantee rather than a runtime test. Document that `debug_assert!` is a no-op in release mode by testing that the validation functions are inlined to nothing. Alternatively, run `cargo test --release` and verify that a test calling `validate_finite(f32::NAN, "x")` does NOT panic in release mode (since `debug_assert!` is compiled out). Mark this test with `#[cfg(not(debug_assertions))]`.

- **`test_validation_error_message_is_descriptive`** -- Call `validate_chunk_coords(99999, 0, 0, 1000)` and catch the panic with `std::panic::catch_unwind`. Extract the panic message and assert it contains: the invalid coordinate value ("99999"), the maximum range ("1000"), and the word "chunk coordinates". This verifies the error message provides actionable debugging information.

- **`test_buffer_size_mismatch_triggers_assert`** -- Call `validate_buffer_size(128, 256, "vertex_buffer")` inside `#[should_panic(expected = "vertex_buffer")]`. Assert the panic message includes both the actual (128) and expected (256) sizes, and the buffer name.

- **`test_all_critical_inputs_have_validation`** -- This is a structural/code-review test rather than a runtime test. Use `grep` or a code search tool to verify that `set_position`, `set_rotation`, `set_voxel`, `get_voxel`, `load_chunk`, and `upload_vertex_buffer` all contain calls to `validate_*` functions or `debug_assert!`. This ensures validation coverage is not accidentally removed during refactoring.

- **`test_index_out_of_bounds_triggers_assert`** -- Call `validate_index(10, 10, "array")` inside `#[should_panic(expected = "out of bounds")]`. The index 10 is not less than length 10, so it must trigger. Also test `validate_index(0, 0, "empty")` which should trigger (index 0 is not less than length 0).

- **`test_range_validation`** -- Call `validate_range(0.5, 0.0, 1.0, "opacity")` and assert it does not panic. Call `validate_range(-0.1, 0.0, 1.0, "opacity")` inside `#[should_panic(expected = "outside valid range")]` and assert it triggers. Call `validate_range(1.1, 0.0, 1.0, "opacity")` and assert it also triggers.
