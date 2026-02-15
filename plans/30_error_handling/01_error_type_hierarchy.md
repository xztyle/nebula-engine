# Error Type Hierarchy

## Problem

A game engine the size of Nebula touches dozens of subsystems -- rendering, networking, voxel storage, physics, audio, asset loading, scripting -- and every one of them can fail in domain-specific ways. Without a structured error hierarchy, error handling degenerates into one of two failure modes:

- **Stringly-typed chaos** -- Functions return `Result<T, String>` or `Box<dyn Error>`, making it impossible to match on specific error variants, test for particular failure modes, or programmatically recover. The caller cannot distinguish "shader failed to compile" from "GPU device lost" without parsing a human-readable message.
- **Monolithic error enum** -- A single flat `EngineError` enum with 50+ variants mixes concerns across domains. A function that only touches voxel storage is forced to acknowledge the existence of `NetworkTimeout` and `ShaderCompilationFailed` in every match arm. The enum grows without bound, and adding a variant to one subsystem forces recompilation of every crate that uses the error type.

What is needed is a layered error hierarchy: each subsystem defines its own narrow error type, and a top-level `NebulaError` delegates to the appropriate domain. The `thiserror` crate (version 2) generates the `std::error::Error` and `Display` implementations, eliminating boilerplate. At application boundaries (the `main()` function, CLI tools, test harnesses), `anyhow` provides ergonomic handling of ad-hoc errors that do not need programmatic matching.

## Solution

### Top-Level Error Enum

Define `NebulaError` in a shared crate (e.g., `nebula-app` or a lightweight `nebula-error` crate if needed to avoid dependency cycles). Each variant wraps a domain-specific error type using `#[from]` for automatic `From` conversion:

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NebulaError {
    #[error(transparent)]
    Render(#[from] RenderError),

    #[error(transparent)]
    Voxel(#[from] VoxelError),

    #[error(transparent)]
    Network(#[from] NetworkError),

    #[error(transparent)]
    Physics(#[from] PhysicsError),

    #[error(transparent)]
    Asset(#[from] AssetError),

    #[error(transparent)]
    Script(#[from] ScriptError),

    #[error(transparent)]
    Io(#[from] IoError),
}
```

The `#[error(transparent)]` attribute means `NebulaError`'s `Display` implementation delegates to the inner error, and `source()` returns the inner error directly. This preserves the full error chain for logging and debugging.

### Domain-Specific Error Types

Each subsystem crate defines its own error enum. These are narrow, containing only variants relevant to that domain:

```rust
// In nebula-render
#[derive(Debug, Error)]
pub enum RenderError {
    #[error("shader compilation failed for '{name}': {reason}")]
    ShaderCompilation { name: String, reason: String },

    #[error("GPU device lost: {0}")]
    DeviceLost(String),

    #[error("surface configuration failed: {0}")]
    SurfaceConfig(String),

    #[error("pipeline creation failed: {0}")]
    PipelineCreation(String),

    #[error("texture creation failed: dimensions {width}x{height}: {reason}")]
    TextureCreation { width: u32, height: u32, reason: String },

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
```

```rust
// In nebula-voxel
#[derive(Debug, Error)]
pub enum VoxelError {
    #[error("chunk at {pos:?} not loaded")]
    ChunkNotLoaded { pos: [i32; 3] },

    #[error("voxel coordinates out of bounds: ({x}, {y}, {z}) in chunk of size {size}")]
    OutOfBounds { x: usize, y: usize, z: usize, size: usize },

    #[error("palette overflow: chunk has more than {max} unique block types")]
    PaletteOverflow { max: usize },

    #[error("chunk deserialization failed: {0}")]
    Deserialization(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
```

```rust
// In nebula-net
#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("connection to {addr} timed out after {timeout_ms}ms")]
    ConnectionTimeout { addr: String, timeout_ms: u64 },

    #[error("connection refused by {addr}")]
    ConnectionRefused { addr: String },

    #[error("protocol version mismatch: local={local}, remote={remote}")]
    ProtocolMismatch { local: u32, remote: u32 },

    #[error("message too large: {size} bytes (max {max})")]
    MessageTooLarge { size: usize, max: usize },

    #[error("disconnected: {reason}")]
    Disconnected { reason: String },

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
```

```rust
// In nebula-physics
#[derive(Debug, Error)]
pub enum PhysicsError {
    #[error("rigid body handle {0:?} is invalid or has been removed")]
    InvalidBodyHandle(u64),

    #[error("collision shape generation failed for mesh with {vertex_count} vertices: {reason}")]
    ShapeGeneration { vertex_count: usize, reason: String },

    #[error("physics world step diverged: dt={dt}, max_substeps={max_substeps}")]
    StepDivergence { dt: f32, max_substeps: u32 },
}
```

```rust
// In nebula-assets
#[derive(Debug, Error)]
pub enum AssetError {
    #[error("asset not found: {path}")]
    NotFound { path: String },

    #[error("unsupported asset format: {extension}")]
    UnsupportedFormat { extension: String },

    #[error("asset loading failed for '{path}': {reason}")]
    LoadFailed { path: String, reason: String },

    #[error("asset decode error for '{path}': {reason}")]
    DecodeError { path: String, reason: String },

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
```

```rust
// In nebula-scripting
#[derive(Debug, Error)]
pub enum ScriptError {
    #[error("script compilation failed: {path}: {message}")]
    CompilationFailed { path: String, message: String },

    #[error("runtime error in script '{path}' at line {line}: {message}")]
    RuntimeError { path: String, line: usize, message: String },

    #[error("script function '{function}' not found in '{path}'")]
    FunctionNotFound { path: String, function: String },

    #[error("script API call failed: {0}")]
    ApiFailed(String),
}
```

```rust
// In nebula-app or a shared crate
#[derive(Debug, Error)]
pub enum IoError {
    #[error("failed to read config file '{path}': {source}")]
    ConfigRead { path: String, source: std::io::Error },

    #[error("failed to write save file '{path}': {source}")]
    SaveWrite { path: String, source: std::io::Error },

    #[error("directory creation failed for '{path}': {source}")]
    DirCreation { path: String, source: std::io::Error },

    #[error(transparent)]
    Raw(#[from] std::io::Error),
}
```

### Application Boundaries with `anyhow`

The `main()` function and CLI tools use `anyhow::Result` as their return type. This allows any error implementing `std::error::Error` to propagate with `?`, and `anyhow` automatically captures the error chain and backtrace:

```rust
use anyhow::{Context, Result};

fn main() -> Result<()> {
    let config = load_config("nebula.toml")
        .context("Failed to load engine configuration")?;

    let gpu = init_gpu(&config)
        .context("GPU initialization failed")?;

    run_game_loop(gpu, config)?;

    Ok(())
}
```

The `.context()` call adds human-readable context to the error chain without losing the original error. This is the right boundary: `anyhow` is used where errors are reported to the user, not where they are programmatically matched.

### Error Conversion Flow

The conversion chain works as follows:

```
std::io::Error  -->  VoxelError::Io  -->  NebulaError::Voxel  -->  anyhow::Error
                     (via #[from])        (via #[from])            (via ? in main)
```

Each `#[from]` attribute generates a `From` implementation, so the `?` operator works seamlessly across boundaries. A function in `nebula-voxel` that does file I/O can use `?` on `std::io::Error`, which auto-converts to `VoxelError::Io`. The caller in `nebula-app` can use `?` again, converting `VoxelError` to `NebulaError::Voxel`. Finally, `main()` converts `NebulaError` to `anyhow::Error` for reporting.

### Downcasting

When a caller receives a `NebulaError` and needs to check for a specific domain error, standard `downcast_ref` works through the `source()` chain:

```rust
fn handle_error(err: &NebulaError) {
    match err {
        NebulaError::Render(render_err) => match render_err {
            RenderError::DeviceLost(_) => {
                tracing::error!("GPU device lost, attempting recovery");
                // Attempt GPU re-initialization
            }
            _ => tracing::error!("Render error: {render_err}"),
        },
        NebulaError::Network(NetworkError::Disconnected { reason }) => {
            tracing::warn!("Disconnected: {reason}, switching to single-player");
        }
        other => tracing::error!("Engine error: {other}"),
    }
}
```

For `anyhow::Error`, downcasting is also supported:

```rust
fn handle_anyhow(err: &anyhow::Error) {
    if let Some(render_err) = err.downcast_ref::<RenderError>() {
        // Handle render-specific error
    }
}
```

## Outcome

Every subsystem crate has its own error enum with descriptive, structured variants. A top-level `NebulaError` in `nebula-app` unifies them via `#[from]` conversion. All error types implement `std::error::Error`, `Display`, and `Debug`. The `?` operator propagates errors across crate boundaries without manual conversion code. `anyhow` is used exclusively at application boundaries for reporting. Callers can match on specific error variants for programmatic recovery or downcast from `anyhow::Error` when needed.

## Demo Integration

**Demo crate:** `nebula-demo`

All errors flow through a structured type hierarchy. The console shows `RenderError::ShaderCompilation { file: "terrain.wgsl", line: 42 }` instead of generic panics.

## Crates & Dependencies

- **`thiserror = "2"`** -- Derive macro for `std::error::Error` implementations. Generates `Display`, `Error::source()`, and `From` implementations from attributes. Zero runtime cost -- it is purely a compile-time code generator.
- **`anyhow = "1"`** -- Ergonomic error handling for application-level code. Provides `anyhow::Result`, `anyhow::Error`, `context()`, and `downcast()`. Used only at application boundaries, never in library crates.

## Unit Tests

- **`test_render_error_display`** -- Construct each `RenderError` variant and call `.to_string()`. Assert that `RenderError::ShaderCompilation { name: "terrain.wgsl".into(), reason: "missing entry point".into() }` produces `"shader compilation failed for 'terrain.wgsl': missing entry point"`. Repeat for `DeviceLost`, `SurfaceConfig`, `PipelineCreation`, and `TextureCreation` variants.

- **`test_voxel_error_display`** -- Construct `VoxelError::OutOfBounds { x: 33, y: 0, z: 15, size: 32 }` and assert the display string contains "out of bounds" and the coordinates. Construct `VoxelError::PaletteOverflow { max: 256 }` and assert it mentions the max value.

- **`test_network_error_display`** -- Construct `NetworkError::ConnectionTimeout { addr: "127.0.0.1:9000".into(), timeout_ms: 5000 }` and assert the display string contains the address and timeout value.

- **`test_error_chain_preserves_source`** -- Create a `std::io::Error` with `ErrorKind::NotFound`. Convert it into `VoxelError::Io` via `From`. Call `.source()` on the `VoxelError` and assert it returns `Some`. Downcast the source to `std::io::Error` and verify the kind is `NotFound`. This validates that `#[from]` correctly wires up the `source()` chain.

- **`test_thiserror_derives_error_trait`** -- Verify that `RenderError`, `VoxelError`, `NetworkError`, `PhysicsError`, `AssetError`, `ScriptError`, and `IoError` all implement `std::error::Error`. This can be done with a generic function `fn assert_error<T: std::error::Error>()` called for each type.

- **`test_from_conversion_domain_to_nebula`** -- Create a `RenderError::DeviceLost("test".into())` and convert it to `NebulaError` using `NebulaError::from(err)`. Assert the result matches `NebulaError::Render(_)`. Repeat for each domain error type to verify all `From` implementations are generated.

- **`test_question_mark_propagation`** -- Write a helper function returning `Result<(), NebulaError>` that calls a function returning `Result<(), VoxelError>` with `?`. Pass in an `Err(VoxelError::ChunkNotLoaded { pos: [0, 0, 0] })` and verify the outer result is `Err(NebulaError::Voxel(_))`. This validates that `?` triggers the `From` conversion.

- **`test_downcast_from_nebula_error`** -- Create a `NebulaError::Render(RenderError::DeviceLost("gpu crash".into()))`. Match on the variant, extract the inner `RenderError`, and assert it is `RenderError::DeviceLost`. This validates that pattern matching works for programmatic recovery.

- **`test_anyhow_downcast`** -- Convert a `RenderError::DeviceLost("test".into())` into an `anyhow::Error`. Call `downcast_ref::<RenderError>()` and assert it returns `Some`. Verify the downcast value matches the original error.

- **`test_anyhow_context_preserves_chain`** -- Create a `VoxelError::ChunkNotLoaded { pos: [1, 2, 3] }`, wrap it in `anyhow::Error`, add `.context("failed during world load")`. Call `.to_string()` on the `anyhow::Error` and assert it contains "failed during world load". Call `.source()` and verify the chain includes the original `VoxelError`.
