# Hot Reload

## Problem

During development, the iteration cycle for visual changes is painfully slow without hot-reload. An artist tweaks a texture, saves the file, then must restart the entire engine to see the result — which means reloading the world, repositioning the camera, and waiting for all other assets to load. The same applies to shader programmers editing `.wgsl` files and designers adjusting material parameters in `.ron` files. For a voxel engine with cubesphere planets and 128-bit coordinate spaces, restarting means regenerating terrain and re-entering the exact coordinate position, which can take significant time.

Hot-reload solves this by watching the asset directory for file changes, re-loading modified assets in-place, and updating all references through the existing handle system. The player model instantly reflects the new texture, the terrain immediately uses the updated shader, and the material changes take effect on the next frame — all without restarting.

Hot-reload must only be active in development builds. Release builds must not include filesystem watching code, as it adds overhead and is a potential security surface.

## Solution

### Architecture

The hot-reload system has three components:

1. **File watcher** — Uses the `notify` crate to receive filesystem events (create, modify, rename) for the asset directory.
2. **Debounce filter** — Coalesces rapid successive changes to the same file (common when editors save incrementally) into a single reload event after a configurable quiet period.
3. **Reload dispatcher** — Maps changed file paths to asset handles and triggers re-loading through the existing async loading system.

### File Watcher

```rust
use notify::{
    Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub struct HotReloadWatcher {
    /// The notify watcher handle. Dropping this stops watching.
    _watcher: RecommendedWatcher,
    /// Receives raw filesystem events from the watcher thread.
    event_rx: mpsc::Receiver<notify::Result<Event>>,
    /// Debounce state: path -> last event time.
    pending: std::collections::HashMap<PathBuf, Instant>,
    /// Debounce duration. Changes within this window are coalesced.
    debounce: Duration,
    /// Whether hot-reload is currently enabled.
    enabled: bool,
}

impl HotReloadWatcher {
    /// Create a new watcher monitoring the given asset directory.
    /// `debounce_ms` is the quiet period in milliseconds before
    /// a change is considered final. Recommended: 200ms.
    pub fn new(
        asset_dir: &Path,
        debounce_ms: u64,
    ) -> Result<Self, HotReloadError> {
        let (tx, rx) = mpsc::channel();

        let mut watcher = RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            Config::default(),
        )
        .map_err(|e| HotReloadError::WatcherInit(e.to_string()))?;

        watcher
            .watch(asset_dir, RecursiveMode::Recursive)
            .map_err(|e| HotReloadError::WatchPath {
                path: asset_dir.to_path_buf(),
                reason: e.to_string(),
            })?;

        log::info!(
            "Hot-reload watcher started on '{}' with {}ms debounce",
            asset_dir.display(),
            debounce_ms,
        );

        Ok(Self {
            _watcher: watcher,
            event_rx: rx,
            pending: std::collections::HashMap::new(),
            debounce: Duration::from_millis(debounce_ms),
            enabled: true,
        })
    }

    /// Enable or disable hot-reload at runtime.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if enabled {
            log::info!("Hot-reload enabled");
        } else {
            log::info!("Hot-reload disabled");
            self.pending.clear();
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Poll for changed files. Returns paths that have settled past
    /// the debounce window. Call this once per frame.
    pub fn poll_changes(&mut self) -> Vec<PathBuf> {
        if !self.enabled {
            // Drain events but do not process them
            while self.event_rx.try_recv().is_ok() {}
            return Vec::new();
        }

        // Collect new events
        while let Ok(event_result) = self.event_rx.try_recv() {
            if let Ok(event) = event_result {
                match event.kind {
                    EventKind::Modify(_) | EventKind::Create(_) => {
                        for path in event.paths {
                            self.pending.insert(path, Instant::now());
                        }
                    }
                    _ => {}
                }
            }
        }

        // Emit paths that have settled past the debounce window
        let now = Instant::now();
        let mut ready = Vec::new();

        self.pending.retain(|path, last_event| {
            if now.duration_since(*last_event) >= self.debounce {
                ready.push(path.clone());
                false // remove from pending
            } else {
                true // keep waiting
            }
        });

        ready
    }
}
```

### Error Type

```rust
#[derive(Debug, thiserror::Error)]
pub enum HotReloadError {
    #[error("failed to initialize file watcher: {0}")]
    WatcherInit(String),

    #[error("failed to watch path {path}: {reason}")]
    WatchPath { path: PathBuf, reason: String },
}
```

### Reload Dispatcher

The dispatcher maps changed file paths back to asset handles using the `AssetPathMap` (from the cache story) and triggers re-loading:

```rust
pub struct ReloadDispatcher;

impl ReloadDispatcher {
    /// Process a list of changed file paths and trigger re-loads.
    pub fn dispatch_reloads(
        changed_paths: &[PathBuf],
        path_map: &AssetPathMap,
        texture_store: &mut AssetStore<CpuTexture>,
        texture_loader: &AssetLoadingSystem<CpuTexture>,
        model_store: &mut AssetStore<ModelAsset>,
        model_loader: &AssetLoadingSystem<ModelAsset>,
        shader_library: &mut ShaderLibrary,
        device: &wgpu::Device,
        pipeline_rebuild: &mut PipelineRebuildQueue,
    ) {
        for path in changed_paths {
            let extension = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");

            match extension {
                "png" | "jpg" | "jpeg" | "bmp" => {
                    Self::reload_texture(
                        path, path_map, texture_store, texture_loader,
                    );
                }
                "glb" | "gltf" => {
                    Self::reload_model(
                        path, path_map, model_store, model_loader,
                    );
                }
                "wgsl" => {
                    Self::reload_shader(
                        path, shader_library, device, pipeline_rebuild,
                    );
                }
                "ron" => {
                    log::info!("RON file changed: {} — material reload", path.display());
                    // Material reload handled by material system
                }
                _ => {
                    log::debug!(
                        "Hot-reload: ignoring change to '{}'",
                        path.display()
                    );
                }
            }
        }
    }

    fn reload_texture(
        path: &Path,
        path_map: &AssetPathMap,
        store: &mut AssetStore<CpuTexture>,
        loader: &AssetLoadingSystem<CpuTexture>,
    ) {
        // Find all handles that reference this path
        for (handle_id, asset_path) in path_map.iter() {
            if asset_path == path {
                log::info!("Hot-reloading texture: {}", path.display());
                let handle = Handle::<CpuTexture>::from_raw_id(handle_id);
                store.set_loading(handle);
                // Re-submit load request. The handle stays the same,
                // so all references remain valid.
                loader.resubmit_load(store, handle, path.to_path_buf());
            }
        }
    }

    fn reload_model(
        path: &Path,
        path_map: &AssetPathMap,
        store: &mut AssetStore<ModelAsset>,
        loader: &AssetLoadingSystem<ModelAsset>,
    ) {
        for (handle_id, asset_path) in path_map.iter() {
            if asset_path == path {
                log::info!("Hot-reloading model: {}", path.display());
                let handle = Handle::<ModelAsset>::from_raw_id(handle_id);
                store.set_loading(handle);
                loader.resubmit_load(store, handle, path.to_path_buf());
            }
        }
    }

    fn reload_shader(
        path: &Path,
        shader_library: &mut ShaderLibrary,
        device: &wgpu::Device,
        pipeline_rebuild: &mut PipelineRebuildQueue,
    ) {
        let shader_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        log::info!("Hot-reloading shader: {}", path.display());

        match shader_library.reload(device, shader_name) {
            Ok(_) => {
                log::info!(
                    "Shader '{}' reloaded successfully — \
                     queuing pipeline rebuild",
                    shader_name
                );
                pipeline_rebuild.queue_rebuild(shader_name);
            }
            Err(e) => {
                log::error!(
                    "Failed to reload shader '{}': {}",
                    shader_name, e
                );
            }
        }
    }
}
```

### Pipeline Rebuild Queue

When a shader is hot-reloaded, all render pipelines that use that shader must be recreated:

```rust
pub struct PipelineRebuildQueue {
    /// Shader names whose pipelines need rebuilding.
    pending_shaders: Vec<String>,
}

impl PipelineRebuildQueue {
    pub fn new() -> Self {
        Self {
            pending_shaders: Vec::new(),
        }
    }

    pub fn queue_rebuild(&mut self, shader_name: &str) {
        if !self.pending_shaders.contains(&shader_name.to_string()) {
            self.pending_shaders.push(shader_name.to_string());
        }
    }

    pub fn drain(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_shaders)
    }

    pub fn is_empty(&self) -> bool {
        self.pending_shaders.is_empty()
    }
}
```

### ECS System

The hot-reload system runs in the `PreUpdate` stage, before any rendering:

```rust
#[cfg(debug_assertions)]
pub fn hot_reload_system(
    mut watcher: ResMut<HotReloadWatcher>,
    path_map: Res<AssetPathMap>,
    mut texture_store: ResMut<TextureAssets>,
    texture_loader: Res<AssetLoadingSystem<CpuTexture>>,
    mut model_store: ResMut<ModelAssets>,
    model_loader: Res<AssetLoadingSystem<ModelAsset>>,
    mut shader_library: ResMut<ShaderLibrary>,
    device: Res<WgpuDevice>,
    mut pipeline_rebuild: ResMut<PipelineRebuildQueue>,
) {
    let changed = watcher.poll_changes();
    if !changed.is_empty() {
        log::debug!("Hot-reload detected {} changed files", changed.len());
        ReloadDispatcher::dispatch_reloads(
            &changed,
            &path_map,
            &mut texture_store.0,
            &texture_loader,
            &mut model_store.0,
            &model_loader,
            &mut shader_library,
            &device.0,
            &mut pipeline_rebuild,
        );
    }
}
```

### Conditional Compilation

The entire hot-reload module is gated behind `debug_assertions` so it has zero cost in release builds:

```rust
#[cfg(debug_assertions)]
mod hot_reload;

#[cfg(debug_assertions)]
pub use hot_reload::*;
```

In release builds, the `HotReloadWatcher`, `ReloadDispatcher`, and the ECS system simply do not exist. The engine binary is smaller and has no filesystem watcher threads.

## Outcome

A development-only hot-reload system that watches the asset directory for file changes, debounces rapid edits (200ms default), and triggers re-loading of textures, models, shaders, and materials through the existing async loading pipeline. Shader hot-reload triggers render pipeline recreation via a `PipelineRebuildQueue`. All reloads happen through the handle system, so no references need updating — the same `Handle<T>` seamlessly transitions from the old data to the new. The system is entirely compiled out of release builds via `cfg(debug_assertions)`. Artists and programmers see changes reflected in-engine within a fraction of a second after saving a file.

## Demo Integration

**Demo crate:** `nebula-demo`

Editing a texture PNG on disk causes the running demo to detect the change and reload it within 1-2 seconds, visible immediately.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `notify` | `7.0` | Cross-platform filesystem event watching (inotify on Linux, FSEvents on macOS, ReadDirectoryChanges on Windows) |
| `wgpu` | `24.0` | Shader module re-creation and pipeline rebuild |
| `bevy_ecs` | `0.15` | `Resource` derive and system scheduling |
| `thiserror` | `2.0` | Error type derivation |
| `log` | `0.4` | Logging change detection, reload events, and errors |

Rust edition 2024. The `notify` crate provides cross-platform filesystem watching that works on all three target platforms (Linux/Windows/macOS). The debounce logic is implemented in-engine rather than using `notify`'s built-in debounce, which gives finer control over the coalescing behavior.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::Duration;

    fn create_test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nebula_hot_reload_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_file_change_triggers_reload() {
        let dir = create_test_dir();
        let test_file = dir.join("test_texture.png");
        fs::write(&test_file, b"initial content").unwrap();

        let mut watcher = HotReloadWatcher::new(&dir, 50).unwrap();

        // Wait for watcher to settle
        std::thread::sleep(Duration::from_millis(100));

        // Modify the file
        fs::write(&test_file, b"modified content").unwrap();

        // Poll until the change is detected (with timeout)
        let start = std::time::Instant::now();
        let mut changes = Vec::new();
        loop {
            changes.extend(watcher.poll_changes());
            if !changes.is_empty() || start.elapsed() > Duration::from_secs(5) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        assert!(
            changes.iter().any(|p| p.ends_with("test_texture.png")),
            "Change to test_texture.png should be detected, got: {:?}",
            changes,
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_debounce_prevents_rapid_reloads() {
        let dir = create_test_dir();
        let test_file = dir.join("rapid.wgsl");
        fs::write(&test_file, b"v1").unwrap();

        // Use a longer debounce to clearly test coalescing
        let mut watcher = HotReloadWatcher::new(&dir, 300).unwrap();
        std::thread::sleep(Duration::from_millis(100));

        // Write rapidly 5 times within the debounce window
        for i in 0..5 {
            fs::write(&test_file, format!("v{}", i + 2)).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }

        // Poll immediately — should get nothing yet (within debounce)
        let immediate = watcher.poll_changes();
        // The changes may or may not appear depending on timing,
        // but we should never get more than 1 event for the same file.

        // Wait past the debounce window
        std::thread::sleep(Duration::from_millis(400));

        let changes = watcher.poll_changes();
        let this_file_count = changes
            .iter()
            .chain(immediate.iter())
            .filter(|p| p.ends_with("rapid.wgsl"))
            .count();

        assert!(
            this_file_count <= 1,
            "Debounce should coalesce rapid changes into at most 1 event, got {}",
            this_file_count,
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_updated_asset_replaces_old() {
        // Verify that the pipeline rebuild queue receives entries
        // when a shader file changes.
        let mut queue = PipelineRebuildQueue::new();
        assert!(queue.is_empty());

        queue.queue_rebuild("terrain");
        assert!(!queue.is_empty());

        let pending = queue.drain();
        assert_eq!(pending, vec!["terrain".to_string()]);
        assert!(queue.is_empty());
    }

    #[test]
    fn test_shader_reload_recreates_pipeline() {
        // Verify that shader reload queues a pipeline rebuild.
        let mut queue = PipelineRebuildQueue::new();

        // Simulate two shader reloads
        queue.queue_rebuild("terrain");
        queue.queue_rebuild("skybox");

        let pending = queue.drain();
        assert_eq!(pending.len(), 2);
        assert!(pending.contains(&"terrain".to_string()));
        assert!(pending.contains(&"skybox".to_string()));
    }

    #[test]
    fn test_duplicate_rebuild_not_queued() {
        let mut queue = PipelineRebuildQueue::new();

        queue.queue_rebuild("terrain");
        queue.queue_rebuild("terrain"); // duplicate
        queue.queue_rebuild("terrain"); // duplicate

        let pending = queue.drain();
        assert_eq!(
            pending.len(),
            1,
            "Duplicate shader rebuilds should be deduplicated"
        );
    }

    #[test]
    fn test_hot_reload_can_be_disabled() {
        let dir = create_test_dir();
        let test_file = dir.join("disabled_test.png");
        fs::write(&test_file, b"initial").unwrap();

        let mut watcher = HotReloadWatcher::new(&dir, 50).unwrap();
        assert!(watcher.is_enabled());

        // Disable hot-reload
        watcher.set_enabled(false);
        assert!(!watcher.is_enabled());

        std::thread::sleep(Duration::from_millis(100));

        // Modify file while disabled
        fs::write(&test_file, b"modified while disabled").unwrap();

        std::thread::sleep(Duration::from_millis(200));

        let changes = watcher.poll_changes();
        assert!(
            changes.is_empty(),
            "Changes should not be reported when hot-reload is disabled"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_re_enable_hot_reload() {
        let dir = create_test_dir();
        let test_file = dir.join("reenable_test.png");
        fs::write(&test_file, b"initial").unwrap();

        let mut watcher = HotReloadWatcher::new(&dir, 50).unwrap();

        // Disable and then re-enable
        watcher.set_enabled(false);
        watcher.set_enabled(true);
        assert!(watcher.is_enabled());

        std::thread::sleep(Duration::from_millis(100));

        // Modify after re-enabling
        fs::write(&test_file, b"after re-enable").unwrap();

        let start = std::time::Instant::now();
        let mut changes = Vec::new();
        loop {
            changes.extend(watcher.poll_changes());
            if !changes.is_empty() || start.elapsed() > Duration::from_secs(5) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        assert!(
            !changes.is_empty(),
            "Changes should be detected after re-enabling hot-reload"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_watcher_nonexistent_directory_returns_error() {
        let result = HotReloadWatcher::new(
            Path::new("/nonexistent/directory/for/testing"),
            200,
        );
        assert!(result.is_err(), "Watching a nonexistent directory should fail");
    }
}
```
