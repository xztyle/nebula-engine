# Shader Module Loading

## Problem

Shaders are the programs that run on the GPU, and they change frequently during development. wgpu uses WGSL (WebGPU Shading Language) as its native shader format. Without a centralized loading system, shaders end up scattered as inline strings throughout the codebase, making iteration slow and error-prone. Developers need hot-reload during development — edit a `.wgsl` file, save, and see the result without restarting the engine. But release builds should embed shaders into the binary via `include_str!` so there are no external file dependencies. Additionally, shader compilation errors must be caught at load time with meaningful diagnostics, not at draw time where they cause panics.

## Solution

### ShaderLibrary

A `ShaderLibrary` struct that serves as the central registry for all compiled shader modules:

```rust
pub struct ShaderLibrary {
    modules: HashMap<String, Arc<wgpu::ShaderModule>>,
    shader_dir: Option<PathBuf>,
}
```

The `Arc<wgpu::ShaderModule>` enables sharing shader modules across multiple pipelines without duplication. The `shader_dir` is `Some` in development (for file-based loading) and `None` in release (embedded-only).

### Loading API

```rust
impl ShaderLibrary {
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
            shader_dir: None,
        }
    }

    /// Set the directory to load .wgsl files from (development mode).
    pub fn with_shader_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.shader_dir = Some(dir.into());
        self
    }

    /// Load a shader from a WGSL source string.
    pub fn load_from_source(
        &mut self,
        device: &wgpu::Device,
        name: &str,
        source: &str,
    ) -> Result<Arc<wgpu::ShaderModule>, ShaderError> { ... }

    /// Load a shader from a file in the shader directory.
    pub fn load_from_file(
        &mut self,
        device: &wgpu::Device,
        name: &str,
        filename: &str,
    ) -> Result<Arc<wgpu::ShaderModule>, ShaderError> { ... }

    /// Get a previously loaded shader by name.
    pub fn get(&self, name: &str) -> Option<Arc<wgpu::ShaderModule>> { ... }

    /// Reload a shader from its file (hot-reload).
    pub fn reload(
        &mut self,
        device: &wgpu::Device,
        name: &str,
    ) -> Result<Arc<wgpu::ShaderModule>, ShaderError> { ... }

    /// Number of loaded shaders.
    pub fn len(&self) -> usize { ... }

    pub fn is_empty(&self) -> bool { ... }
}
```

### Shader Compilation and Validation

When `load_from_source` is called:

1. Create a `wgpu::ShaderModuleDescriptor` with the provided WGSL source.
2. Call `device.create_shader_module(descriptor)`. wgpu validates the WGSL at this point — if the shader has syntax errors or type errors, wgpu will log a validation error.
3. To catch errors before they become panics, use `device.push_error_scope(wgpu::ErrorFilter::Validation)` before creation and `device.pop_error_scope()` after. If an error is captured, return `ShaderError::CompilationFailed { name, message }`.
4. If compilation succeeds, wrap the module in `Arc` and insert into the `modules` map under the given name.
5. If a module with the same name already exists, replace it. The old `Arc` is dropped, and existing pipelines holding the old `Arc` continue to work until they are recreated.

### Development vs Release Loading

```rust
/// Macro for embedding shaders in release, loading from file in debug.
#[macro_export]
macro_rules! load_shader {
    ($library:expr, $device:expr, $name:expr, $file:expr) => {
        if cfg!(debug_assertions) {
            $library.load_from_file($device, $name, $file)
        } else {
            $library.load_from_source($device, $name, include_str!($file))
        }
    };
}
```

In debug builds, shaders are read from disk at runtime, enabling hot-reload with the `reload()` method. A file watcher (future story) can call `reload()` when a `.wgsl` file changes.

In release builds, `include_str!` embeds the shader source at compile time. No filesystem access is needed at runtime.

### Error Type

```rust
#[derive(Debug, thiserror::Error)]
pub enum ShaderError {
    #[error("shader '{name}' failed to compile: {message}")]
    CompilationFailed { name: String, message: String },

    #[error("shader file not found: {path}")]
    FileNotFound { path: PathBuf },

    #[error("failed to read shader file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("shader '{name}' not found in library")]
    NotLoaded { name: String },

    #[error("no shader directory configured for file-based loading")]
    NoShaderDir,
}
```

### Hot-Reload Flow

1. A file watcher detects that `assets/shaders/terrain.wgsl` has changed.
2. It calls `shader_library.reload(&device, "terrain")`.
3. `reload()` reads the file again, compiles the new source, and replaces the cached module.
4. Any system holding the old `Arc<ShaderModule>` still works until its pipeline is recreated.
5. A pipeline rebuild is triggered for pipelines that reference the reloaded shader.

Pipeline rebuild on shader reload is handled by a separate system — this story only covers shader loading and caching.

## Outcome

A `ShaderLibrary` that compiles, caches, and serves WGSL shader modules by name. Development builds load from disk with hot-reload capability. Release builds embed shaders into the binary. Compilation errors are caught at load time and reported with the shader name and error message. Downstream pipeline creation code calls `shader_library.get("terrain")` and receives an `Arc<ShaderModule>` ready for use.

## Demo Integration

**Demo crate:** `nebula-demo`

The unlit vertex and fragment shaders are compiled from WGSL source. The console logs `Compiled shader: unlit.wgsl`. No visible change yet.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | Shader module creation and validation |
| `thiserror` | `2.0` | Error type derivation |
| `log` | `0.4` | Logging shader load/reload events |

No additional dependencies. File I/O uses `std::fs`. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const VALID_SHADER: &str = r#"
        @vertex
        fn vs_main(@builtin(vertex_index) idx: u32) -> @builtin(position) vec4<f32> {
            return vec4<f32>(0.0, 0.0, 0.0, 1.0);
        }

        @fragment
        fn fs_main() -> @location(0) vec4<f32> {
            return vec4<f32>(1.0, 0.0, 0.0, 1.0);
        }
    "#;

    const INVALID_SHADER: &str = r#"
        @vertex
        fn vs_main() -> @builtin(position) vec4<f32> {
            return undeclared_variable;
        }
    "#;

    #[test]
    fn test_load_valid_shader_succeeds() {
        let device = create_test_device();
        let mut library = ShaderLibrary::new();
        let result = library.load_from_source(&device, "test", VALID_SHADER);
        assert!(result.is_ok());
    }

    #[test]
    fn test_load_invalid_shader_returns_error() {
        let device = create_test_device();
        let mut library = ShaderLibrary::new();
        let result = library.load_from_source(&device, "bad", INVALID_SHADER);
        assert!(result.is_err());
        match result.unwrap_err() {
            ShaderError::CompilationFailed { name, .. } => {
                assert_eq!(name, "bad");
            }
            other => panic!("expected CompilationFailed, got {:?}", other),
        }
    }

    #[test]
    fn test_cache_returns_same_module_for_same_name() {
        let device = create_test_device();
        let mut library = ShaderLibrary::new();
        library.load_from_source(&device, "shared", VALID_SHADER).unwrap();

        let a = library.get("shared").unwrap();
        let b = library.get("shared").unwrap();
        // Both Arcs point to the same allocation
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn test_shader_library_starts_empty() {
        let library = ShaderLibrary::new();
        assert!(library.is_empty());
        assert_eq!(library.len(), 0);
    }

    #[test]
    fn test_get_nonexistent_shader_returns_none() {
        let library = ShaderLibrary::new();
        assert!(library.get("nonexistent").is_none());
    }

    #[test]
    fn test_load_from_file_without_shader_dir_returns_error() {
        let device = create_test_device();
        let mut library = ShaderLibrary::new(); // no shader_dir set
        let result = library.load_from_file(&device, "test", "test.wgsl");
        assert!(matches!(result, Err(ShaderError::NoShaderDir)));
    }

    #[test]
    fn test_multiple_shaders_coexist() {
        let device = create_test_device();
        let mut library = ShaderLibrary::new();
        library.load_from_source(&device, "shader_a", VALID_SHADER).unwrap();
        library.load_from_source(&device, "shader_b", VALID_SHADER).unwrap();
        assert_eq!(library.len(), 2);
        assert!(library.get("shader_a").is_some());
        assert!(library.get("shader_b").is_some());
    }

    #[test]
    fn test_reload_replaces_cached_module() {
        let device = create_test_device();
        let mut library = ShaderLibrary::new()
            .with_shader_dir("test_shaders/");
        // Initial load from source
        library.load_from_source(&device, "reloadable", VALID_SHADER).unwrap();
        let original = library.get("reloadable").unwrap();

        // Simulate reload with new source
        library.load_from_source(&device, "reloadable", VALID_SHADER).unwrap();
        let reloaded = library.get("reloadable").unwrap();

        // The Arc should point to a different allocation after reload
        assert!(!Arc::ptr_eq(&original, &reloaded));
    }
}
```
