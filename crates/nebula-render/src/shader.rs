//! Shader module loading, caching, and hot-reload system.

use log::{debug, info};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use thiserror::Error;
use wgpu::{ShaderModuleDescriptor, ShaderSource};

/// Error types for shader loading operations.
#[derive(Debug, Error)]
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

/// Central registry for compiled shader modules with hot-reload support.
pub struct ShaderLibrary {
    modules: HashMap<String, Arc<wgpu::ShaderModule>>,
    shader_dir: Option<PathBuf>,
}

impl ShaderLibrary {
    /// Create a new empty shader library.
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
    ) -> Result<Arc<wgpu::ShaderModule>, ShaderError> {
        debug!("Loading shader '{}' from source", name);

        let descriptor = ShaderModuleDescriptor {
            label: Some(name),
            source: ShaderSource::Wgsl(source.into()),
        };

        // Create shader module and handle potential errors via device validation
        let module = device.create_shader_module(descriptor);

        let arc_module = Arc::new(module);
        let replaced = self
            .modules
            .insert(name.to_string(), arc_module.clone())
            .is_some();

        if replaced {
            info!("Replaced shader '{}'", name);
        } else {
            info!("Loaded shader '{}'", name);
        }

        Ok(arc_module)
    }

    /// Load a shader from a file in the shader directory.
    pub fn load_from_file(
        &mut self,
        device: &wgpu::Device,
        name: &str,
        filename: &str,
    ) -> Result<Arc<wgpu::ShaderModule>, ShaderError> {
        let shader_dir = self.shader_dir.as_ref().ok_or(ShaderError::NoShaderDir)?;
        let path = shader_dir.join(filename);

        debug!("Loading shader '{}' from file: {:?}", name, path);

        if !path.exists() {
            return Err(ShaderError::FileNotFound { path });
        }

        let source = std::fs::read_to_string(&path)?;
        self.load_from_source(device, name, &source)
    }

    /// Get a previously loaded shader by name.
    pub fn get(&self, name: &str) -> Option<Arc<wgpu::ShaderModule>> {
        self.modules.get(name).cloned()
    }

    /// Reload a shader from its file (hot-reload).
    /// This assumes the shader was originally loaded from a file.
    pub fn reload(
        &mut self,
        device: &wgpu::Device,
        name: &str,
    ) -> Result<Arc<wgpu::ShaderModule>, ShaderError> {
        let shader_dir = self.shader_dir.as_ref().ok_or(ShaderError::NoShaderDir)?;

        // We need to find the original filename. For now, we assume it's name + ".wgsl"
        // In a more sophisticated system, we'd track the original filename per shader
        let filename = format!("{}.wgsl", name);
        let path = shader_dir.join(&filename);

        if !path.exists() {
            return Err(ShaderError::FileNotFound { path });
        }

        info!("Reloading shader '{}'", name);
        let source = std::fs::read_to_string(&path)?;
        self.load_from_source(device, name, &source)
    }

    /// Number of loaded shaders.
    pub fn len(&self) -> usize {
        self.modules.len()
    }

    /// Check if the shader library is empty.
    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }
}

impl Default for ShaderLibrary {
    fn default() -> Self {
        Self::new()
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::{
        Backends, Device, DeviceDescriptor, Features, Instance, InstanceDescriptor, Limits,
        RequestAdapterOptions,
    };

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

    fn create_test_device() -> Option<Device> {
        let instance = Instance::new(&InstanceDescriptor {
            backends: Backends::all(),
            ..Default::default()
        });

        let adapter =
            pollster::block_on(instance.request_adapter(&RequestAdapterOptions::default()))?;

        let (device, _queue) = pollster::block_on(adapter.request_device(&DeviceDescriptor {
            label: None,
            required_features: Features::empty(),
            required_limits: Limits::default(),
            memory_hints: Default::default(),
            experimental_features: Default::default(),
            trace: Default::default(),
        }))
        .ok()?;

        Some(device)
    }

    #[test]
    fn test_load_valid_shader_succeeds() {
        let Some(device) = create_test_device() else {
            return;
        };
        let mut library = ShaderLibrary::new();
        let result = library.load_from_source(&device, "test", VALID_SHADER);
        assert!(result.is_ok());
    }

    #[test]
    #[should_panic(expected = "Validation Error")]
    fn test_load_invalid_shader_panics() {
        let Some(device) = create_test_device() else {
            return;
        };
        let mut library = ShaderLibrary::new();
        let _result = library.load_from_source(&device, "bad", INVALID_SHADER);
        // This should panic due to shader compilation error
    }

    #[test]
    fn test_cache_returns_same_module_for_same_name() {
        let Some(device) = create_test_device() else {
            return;
        };
        let mut library = ShaderLibrary::new();
        library
            .load_from_source(&device, "shared", VALID_SHADER)
            .unwrap();

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
        let Some(device) = create_test_device() else {
            return;
        };
        let mut library = ShaderLibrary::new(); // no shader_dir set
        let result = library.load_from_file(&device, "test", "test.wgsl");
        assert!(matches!(result, Err(ShaderError::NoShaderDir)));
    }

    #[test]
    fn test_multiple_shaders_coexist() {
        let Some(device) = create_test_device() else {
            return;
        };
        let mut library = ShaderLibrary::new();
        library
            .load_from_source(&device, "shader_a", VALID_SHADER)
            .unwrap();
        library
            .load_from_source(&device, "shader_b", VALID_SHADER)
            .unwrap();
        assert_eq!(library.len(), 2);
        assert!(library.get("shader_a").is_some());
        assert!(library.get("shader_b").is_some());
    }

    #[test]
    fn test_reload_replaces_cached_module() {
        let Some(device) = create_test_device() else {
            return;
        };
        let mut library = ShaderLibrary::new().with_shader_dir("test_shaders/");
        // Initial load from source
        library
            .load_from_source(&device, "reloadable", VALID_SHADER)
            .unwrap();
        let original = library.get("reloadable").unwrap();

        // Simulate reload with new source
        library
            .load_from_source(&device, "reloadable", VALID_SHADER)
            .unwrap();
        let reloaded = library.get("reloadable").unwrap();

        // The Arc should point to a different allocation after reload
        assert!(!Arc::ptr_eq(&original, &reloaded));
    }
}
