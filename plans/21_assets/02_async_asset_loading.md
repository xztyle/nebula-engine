# Async Asset Loading

## Problem

Loading assets from disk is slow — reading a 4K texture takes milliseconds, parsing a glTF model with thousands of vertices takes tens of milliseconds, and decompressing audio can take even longer. If the main thread blocks on these operations, the game stutters or freezes. Players see dropped frames during level transitions, entering new biomes, or approaching unloaded chunks on a cubesphere planet. The problem compounds when dozens of assets must be loaded simultaneously (e.g., transitioning from a menu to gameplay).

The engine needs to submit load requests from the main thread, receive a `Handle<T>` immediately (in the `Pending` state), and have background threads perform the actual disk I/O and parsing. When the background work completes, the main thread must be notified so it can finalize the asset — particularly for GPU resources like textures and meshes that can only be uploaded from the thread that owns the wgpu `Device` and `Queue`.

## Solution

### Architecture Overview

The async loading system has three layers:

1. **Request submission** — The main thread calls `load_asset()`, which allocates a `Handle<T>`, marks it as `Loading` in the `AssetStore<T>`, and sends a load request to the task pool.
2. **Background processing** — A pool of worker threads picks up the request, reads the file, decodes/parses the data into a CPU-side intermediate form, and sends the result back through a channel.
3. **Main-thread finalization** — Each frame, the main thread drains the completion channel, stores the loaded data in `AssetStore<T>`, and queues GPU uploads for assets that need them (textures, meshes).

### Load Request and Completion Types

```rust
use std::path::PathBuf;

/// A request to load an asset, sent from the main thread to the task pool.
pub struct LoadRequest<T> {
    pub handle: Handle<T>,
    pub path: PathBuf,
    pub loader: Box<dyn AssetLoader<T> + Send>,
}

/// The result of a background load, sent from the task pool to the main thread.
pub enum LoadResult<T: Send + 'static> {
    Success {
        handle: Handle<T>,
        data: T,
    },
    Failure {
        handle: Handle<T>,
        error: String,
    },
}
```

### AssetLoader Trait

Each asset type implements a loader trait that describes how to read raw bytes and convert them into the engine's internal representation:

```rust
pub trait AssetLoader<T>: Send + 'static {
    /// Load an asset from raw bytes read from disk.
    /// This runs on a background thread — it must not access GPU resources.
    fn load(&self, bytes: &[u8], path: &std::path::Path) -> Result<T, AssetLoadError>;
}

#[derive(Debug, thiserror::Error)]
pub enum AssetLoadError {
    #[error("file not found: {path}")]
    FileNotFound { path: PathBuf },

    #[error("I/O error reading {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to decode asset at {path}: {reason}")]
    DecodeFailed { path: PathBuf, reason: String },
}
```

### AssetLoadingSystem

The central coordinator that owns the channels and drives the load lifecycle:

```rust
use std::sync::mpsc;
use tokio::runtime::Runtime;

pub struct AssetLoadingSystem<T: Send + 'static> {
    /// Send load requests to background tasks.
    request_tx: mpsc::Sender<LoadRequest<T>>,
    /// Receive completed loads on the main thread.
    completion_rx: mpsc::Receiver<LoadResult<T>>,
    /// Tokio runtime for spawning async file I/O tasks.
    runtime: Arc<Runtime>,
}

impl<T: Send + 'static> AssetLoadingSystem<T> {
    pub fn new(runtime: Arc<Runtime>, num_workers: usize) -> Self {
        let (request_tx, request_rx) = mpsc::channel::<LoadRequest<T>>();
        let (completion_tx, completion_rx) = mpsc::channel::<LoadResult<T>>();

        let request_rx = Arc::new(Mutex::new(request_rx));

        // Spawn worker tasks that pull from the request channel
        for _ in 0..num_workers {
            let rx = Arc::clone(&request_rx);
            let tx = completion_tx.clone();
            runtime.spawn_blocking(move || {
                loop {
                    let request = {
                        let rx = rx.lock().unwrap();
                        match rx.recv() {
                            Ok(req) => req,
                            Err(_) => return, // channel closed, shut down
                        }
                    };

                    let result = match std::fs::read(&request.path) {
                        Ok(bytes) => {
                            match request.loader.load(&bytes, &request.path) {
                                Ok(data) => LoadResult::Success {
                                    handle: request.handle,
                                    data,
                                },
                                Err(e) => LoadResult::Failure {
                                    handle: request.handle,
                                    error: e.to_string(),
                                },
                            }
                        }
                        Err(e) => LoadResult::Failure {
                            handle: request.handle,
                            error: format!("I/O error: {e}"),
                        },
                    };

                    let _ = tx.send(result);
                }
            });
        }

        Self {
            request_tx,
            completion_rx,
            runtime,
        }
    }

    /// Submit a load request. Returns the handle immediately.
    pub fn request_load(
        &self,
        store: &mut AssetStore<T>,
        path: PathBuf,
        loader: impl AssetLoader<T>,
    ) -> Handle<T> {
        let handle = Handle::new();
        store.insert_pending(handle);
        store.set_loading(handle);

        let request = LoadRequest {
            handle,
            path,
            loader: Box::new(loader),
        };

        self.request_tx.send(request).expect("Worker pool shut down");
        handle
    }

    /// Drain completed loads and update the asset store.
    /// Call this once per frame on the main thread.
    /// Returns a list of handles that finished loading (success or failure).
    pub fn process_completions(
        &self,
        store: &mut AssetStore<T>,
    ) -> Vec<Handle<T>> {
        let mut completed = Vec::new();

        while let Ok(result) = self.completion_rx.try_recv() {
            match result {
                LoadResult::Success { handle, data } => {
                    store.set_loaded(handle, data);
                    log::info!("Asset {:?} loaded successfully", handle);
                    completed.push(handle);
                }
                LoadResult::Failure { handle, error } => {
                    store.set_failed(handle, error.clone());
                    log::error!("Asset {:?} failed to load: {}", handle, error);
                    completed.push(handle);
                }
            }
        }

        completed
    }
}
```

### GPU Upload Queue

Textures and meshes require GPU upload after the CPU-side data is ready. This is handled by a separate queue that the main thread processes after `process_completions`:

```rust
pub struct GpuUploadQueue {
    pending_textures: Vec<Handle<CpuTexture>>,
    pending_meshes: Vec<Handle<CpuMesh>>,
}

impl GpuUploadQueue {
    pub fn new() -> Self {
        Self {
            pending_textures: Vec::new(),
            pending_meshes: Vec::new(),
        }
    }

    pub fn enqueue_texture(&mut self, handle: Handle<CpuTexture>) {
        self.pending_textures.push(handle);
    }

    pub fn enqueue_mesh(&mut self, handle: Handle<CpuMesh>) {
        self.pending_meshes.push(handle);
    }

    /// Process all pending uploads using the wgpu device and queue.
    /// This must run on the main thread (or whichever thread owns the Device).
    pub fn process_uploads(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_store: &AssetStore<CpuTexture>,
        gpu_texture_store: &mut AssetStore<GpuTexture>,
        mesh_store: &AssetStore<CpuMesh>,
        gpu_mesh_store: &mut AssetStore<GpuMesh>,
    ) {
        for handle in self.pending_textures.drain(..) {
            if let Some(cpu_tex) = texture_store.get(handle) {
                let gpu_tex = upload_texture_to_gpu(device, queue, cpu_tex);
                let gpu_handle = Handle::new();
                gpu_texture_store.insert_pending(gpu_handle);
                gpu_texture_store.set_loaded(gpu_handle, gpu_tex);
            }
        }

        for handle in self.pending_meshes.drain(..) {
            if let Some(cpu_mesh) = mesh_store.get(handle) {
                let gpu_mesh = upload_mesh_to_gpu(device, cpu_mesh);
                let gpu_handle = Handle::new();
                gpu_mesh_store.insert_pending(gpu_handle);
                gpu_mesh_store.set_loaded(gpu_handle, gpu_mesh);
            }
        }
    }
}
```

### ECS Integration

The loading system runs as an ECS system in the `PreUpdate` stage:

```rust
pub fn asset_loading_system(
    texture_loader: Res<AssetLoadingSystem<CpuTexture>>,
    mut texture_store: ResMut<TextureAssets>,
    mut gpu_upload: ResMut<GpuUploadQueue>,
) {
    let completed = texture_loader.process_completions(&mut texture_store.0);
    for handle in completed {
        if texture_store.0.state(handle) == Some(&AssetState::Loaded) {
            gpu_upload.enqueue_texture(handle);
        }
    }
}
```

## Outcome

An asynchronous asset loading pipeline where `request_load()` returns a `Handle<T>` instantly and background threads perform file I/O and decoding. The main thread calls `process_completions()` each frame to drain finished results into the `AssetStore<T>`. GPU uploads for textures and meshes are queued and processed on the main thread where the wgpu `Device` is available. Load errors are captured and stored as `AssetState::Failed` on the handle, never causing panics. The system supports concurrent loading of many assets without blocking the render loop.

## Demo Integration

**Demo crate:** `nebula-demo`

Textures and models load on background threads. Placeholder checkerboard textures appear until the real asset arrives.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | `1.49` | Async runtime with `spawn_blocking` for background file I/O |
| `wgpu` | `24.0` | GPU texture/mesh upload on the main thread |
| `thiserror` | `2.0` | Error type derivation for `AssetLoadError` |
| `log` | `0.4` | Logging load completions and failures |
| `bevy_ecs` | `0.15` | Resource types and system registration |

Rust edition 2024. The channel implementation uses `std::sync::mpsc` to avoid adding a dependency on crossbeam for this use case.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    #[derive(Debug, Clone, PartialEq)]
    struct TestAsset {
        content: String,
    }

    struct TestLoader;

    impl AssetLoader<TestAsset> for TestLoader {
        fn load(
            &self,
            bytes: &[u8],
            _path: &std::path::Path,
        ) -> Result<TestAsset, AssetLoadError> {
            let content = String::from_utf8_lossy(bytes).to_string();
            Ok(TestAsset { content })
        }
    }

    struct FailingLoader;

    impl AssetLoader<TestAsset> for FailingLoader {
        fn load(
            &self,
            _bytes: &[u8],
            path: &std::path::Path,
        ) -> Result<TestAsset, AssetLoadError> {
            Err(AssetLoadError::DecodeFailed {
                path: path.to_path_buf(),
                reason: "intentional test failure".into(),
            })
        }
    }

    fn create_test_runtime() -> Arc<Runtime> {
        Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .build()
                .unwrap(),
        )
    }

    #[test]
    fn test_load_request_returns_handle_immediately() {
        let runtime = create_test_runtime();
        let loader_system = AssetLoadingSystem::<TestAsset>::new(runtime, 2);
        let mut store = AssetStore::new();

        // Use a path that does not exist — we only check that the handle
        // is returned immediately, not that the load succeeds.
        let handle = loader_system.request_load(
            &mut store,
            PathBuf::from("/nonexistent/test.asset"),
            TestLoader,
        );

        assert!(handle.id() > 0);
        assert!(store.contains(handle));
        assert_eq!(store.state(handle), Some(&AssetState::Loading));
    }

    #[test]
    fn test_asset_loads_in_background() {
        let runtime = create_test_runtime();
        let loader_system = AssetLoadingSystem::<TestAsset>::new(runtime, 2);
        let mut store = AssetStore::new();

        // Write a temporary test file
        let tmp_dir = std::env::temp_dir().join("nebula_test_async_load");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let test_file = tmp_dir.join("test_asset.txt");
        std::fs::write(&test_file, b"hello world").unwrap();

        let handle = loader_system.request_load(
            &mut store,
            test_file.clone(),
            TestLoader,
        );

        // Poll until completion (with timeout)
        let start = std::time::Instant::now();
        loop {
            let completed = loader_system.process_completions(&mut store);
            if !completed.is_empty() {
                break;
            }
            if start.elapsed() > Duration::from_secs(5) {
                panic!("Asset load timed out");
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(store.state(handle), Some(&AssetState::Loaded));
        assert_eq!(
            store.get(handle),
            Some(&TestAsset { content: "hello world".into() })
        );

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_loaded_callback_fires_via_completion() {
        let runtime = create_test_runtime();
        let loader_system = AssetLoadingSystem::<TestAsset>::new(runtime, 2);
        let mut store = AssetStore::new();

        let tmp_dir = std::env::temp_dir().join("nebula_test_callback");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let test_file = tmp_dir.join("callback_test.txt");
        std::fs::write(&test_file, b"data").unwrap();

        let handle = loader_system.request_load(
            &mut store,
            test_file,
            TestLoader,
        );

        // Wait for completion
        let mut callback_fired = false;
        let start = std::time::Instant::now();
        loop {
            let completed = loader_system.process_completions(&mut store);
            for h in &completed {
                if h.id() == handle.id() {
                    callback_fired = true;
                }
            }
            if callback_fired || start.elapsed() > Duration::from_secs(5) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(callback_fired, "Completion callback should have fired");

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_concurrent_loads_dont_block() {
        let runtime = create_test_runtime();
        let loader_system = AssetLoadingSystem::<TestAsset>::new(runtime, 4);
        let mut store = AssetStore::new();

        let tmp_dir = std::env::temp_dir().join("nebula_test_concurrent");
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let mut handles = Vec::new();
        for i in 0..10 {
            let path = tmp_dir.join(format!("asset_{i}.txt"));
            std::fs::write(&path, format!("content_{i}")).unwrap();
            let h = loader_system.request_load(&mut store, path, TestLoader);
            handles.push(h);
        }

        // All handles should be returned without blocking
        assert_eq!(handles.len(), 10);
        for h in &handles {
            assert!(store.contains(*h));
        }

        // Wait for all to complete
        let start = std::time::Instant::now();
        loop {
            loader_system.process_completions(&mut store);
            let all_done = handles.iter().all(|h| {
                matches!(
                    store.state(*h),
                    Some(&AssetState::Loaded) | Some(&AssetState::Failed(_))
                )
            });
            if all_done || start.elapsed() > Duration::from_secs(10) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        for h in &handles {
            assert_eq!(store.state(*h), Some(&AssetState::Loaded));
        }

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_load_error_produces_failed_state() {
        let runtime = create_test_runtime();
        let loader_system = AssetLoadingSystem::<TestAsset>::new(runtime, 2);
        let mut store = AssetStore::new();

        // Load from a path that does not exist
        let handle = loader_system.request_load(
            &mut store,
            PathBuf::from("/this/path/does/not/exist.asset"),
            TestLoader,
        );

        let start = std::time::Instant::now();
        loop {
            let completed = loader_system.process_completions(&mut store);
            if !completed.is_empty() || start.elapsed() > Duration::from_secs(5) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(
            matches!(store.state(handle), Some(&AssetState::Failed(_))),
            "Missing file should produce Failed state"
        );
    }
}
```
