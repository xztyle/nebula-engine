# Cross-Platform Build Validation

## Problem

Nebula Engine targets Linux, Windows, and macOS as first-class platforms. Platform-specific code lurks in many areas of a game engine:

- **Window creation** — Winit handles most of this, but surface creation for wgpu differs by platform (Wayland/X11 on Linux, Win32 on Windows, Metal on macOS).
- **File paths** — Windows uses backslashes and drive letters (`C:\Users\...`), Unix systems use forward slashes (`/home/...`). Hardcoded path separators cause subtle bugs that only manifest on one OS.
- **GPU surface creation** — The wgpu backend selection differs: Vulkan on Linux, DX12/Vulkan on Windows, Metal on macOS. Surface capabilities and preferred texture formats vary.
- **TCP socket behavior** — Socket options, error codes, and connection behavior have subtle differences across operating systems. `SO_REUSEADDR` behaves differently on Windows vs. Unix.
- **Configuration directories** — Each OS has its own convention for where user configuration, save data, and log files should be stored.
- **Dynamic library loading** — If the engine ever loads plugins, the file extension and loading mechanism differs (`.so`, `.dll`, `.dylib`).

If platform-specific code is scattered throughout the codebase with ad-hoc `#[cfg]` guards, it becomes unmaintainable. A single abstraction point is needed.

## Solution

### Platform Abstraction Module

Create a `nebula-platform` utility module (initially inside `nebula-app`, to be extracted into its own crate if it grows) that provides a unified API for all platform-specific concerns:

```rust
pub struct PlatformDirs {
    pub config_dir: PathBuf,   // User config: config.ron, keybindings
    pub data_dir: PathBuf,     // Save games, world data
    pub cache_dir: PathBuf,    // Shader cache, asset cache
    pub log_dir: PathBuf,      // Log files
}

impl PlatformDirs {
    pub fn resolve() -> Result<Self, PlatformError> {
        let base = dirs::config_dir()
            .ok_or(PlatformError::NoConfigDir)?;
        let app_dir = base.join("nebula-engine");

        Ok(Self {
            config_dir: app_dir.join("config"),
            data_dir: dirs::data_dir()
                .unwrap_or_else(|| app_dir.clone())
                .join("nebula-engine"),
            cache_dir: dirs::cache_dir()
                .unwrap_or_else(|| app_dir.clone())
                .join("nebula-engine"),
            log_dir: app_dir.join("logs"),
        })
    }
}
```

### Expected Directory Locations

| Directory   | Linux                              | Windows                                  | macOS                                       |
|-------------|------------------------------------|------------------------------------------|---------------------------------------------|
| Config      | `~/.config/nebula-engine/config`   | `%APPDATA%\nebula-engine\config`         | `~/Library/Application Support/nebula-engine/config` |
| Data        | `~/.local/share/nebula-engine`     | `%APPDATA%\nebula-engine`                | `~/Library/Application Support/nebula-engine`        |
| Cache       | `~/.cache/nebula-engine`           | `%LOCALAPPDATA%\nebula-engine`           | `~/Library/Caches/nebula-engine`                     |
| Logs        | `~/.config/nebula-engine/logs`     | `%APPDATA%\nebula-engine\logs`           | `~/Library/Application Support/nebula-engine/logs`   |

### cfg Guards Policy

All platform-specific code must follow these rules:

1. **Use `cfg(target_os = "...")`** for OS-specific code, never `cfg(unix)` vs `cfg(windows)` unless truly needed for the Unix family.
2. **Isolate platform code** — Platform-specific implementations go in submodules (`platform/linux.rs`, `platform/windows.rs`, `platform/macos.rs`) behind a common trait or function signature.
3. **Never hardcode path separators** — Always use `PathBuf::join()`, `Path::push()`, or the `path!` macro. Never construct paths with string concatenation using `/` or `\\`.
4. **Always use `PathBuf`** — All path-carrying types use `std::path::PathBuf` and `std::path::Path`, never `String`.

### Build Validation

The CI pipeline (see `02_ci_pipeline.md`) already runs `cargo check --workspace` on all three platforms. This story adds:

1. **Platform-specific integration tests** — Tests gated behind `#[cfg(target_os = "...")]` that validate OS-specific behavior.
2. **A `cargo clippy` lint** — Custom lint configuration to warn on string-based path construction patterns.
3. **Directory creation on startup** — The application creates all required directories on first run with appropriate error handling if permissions are insufficient.

### Handling GPU Backend Differences

```rust
pub fn preferred_backends() -> wgpu::Backends {
    #[cfg(target_os = "linux")]
    { wgpu::Backends::VULKAN }

    #[cfg(target_os = "windows")]
    { wgpu::Backends::DX12 | wgpu::Backends::VULKAN }

    #[cfg(target_os = "macos")]
    { wgpu::Backends::METAL }
}
```

While `wgpu::Backends::all()` works, being explicit about backend preference avoids unexpected fallback to less performant backends (e.g., OpenGL on Linux when Vulkan is available).

## Outcome

`cargo check --workspace` succeeds on all three platforms in CI. Platform-specific directories resolve correctly per OS, and all paths use `PathBuf` with no hardcoded separators. The `PlatformDirs` struct provides a single point of access for all OS-specific directory logic. Adding platform-specific behavior in the future follows the established pattern of isolated modules behind a common interface.

## Demo Integration

**Demo crate:** `nebula-demo`

No visible demo change; confirms `nebula-demo` compiles on Linux, Windows, and macOS. Platform-specific directory logic is exercised at startup.

## Crates & Dependencies

- **`dirs = "6"`** — Cross-platform standard directory resolution. Provides `config_dir()`, `data_dir()`, `cache_dir()`, and other XDG/Known Folder/Library paths appropriate to each OS.

## Unit Tests

- **`test_config_dir_exists`** — Call `dirs::config_dir()` and assert it returns `Some(path)` where `path` is a non-empty `PathBuf`. This test runs on all platforms in CI and validates that the `dirs` crate correctly identifies the OS-specific configuration directory.

- **`test_data_dir_exists`** — Call `dirs::data_dir()` and assert it returns `Some(path)`. On Linux this should be under `~/.local/share`, on Windows under `%APPDATA%`, and on macOS under `~/Library/Application Support`.

- **`test_platform_dirs_resolve`** — Construct a `PlatformDirs` via `PlatformDirs::resolve()` and assert all four directory paths are non-empty and are absolute paths (not relative). Do not assert they exist on disk yet (they are created on first run).

- **`test_paths_use_pathbuf`** — A code-review-level test enforced by Clippy configuration. The project's `.clippy.toml` or lint attributes should warn on patterns that construct paths from raw strings with hardcoded separators.

- **`test_no_hardcoded_separators`** — A CI script (or a Rust test that reads source files) that greps for patterns like `format!("{}\\{}", ...)` or `format!("{}/{}", ...)` in path-construction contexts. This catches accidental hardcoded separators that Clippy might miss.

- **`test_directory_creation`** — Call the directory creation function and assert that all four directories exist on disk afterward. Clean up the directories after the test to avoid polluting the test environment. Use `tempdir` to create an isolated test root.

- **`test_preferred_backends_not_empty`** — Call `preferred_backends()` and assert the result is not `wgpu::Backends::empty()`. This ensures every supported platform has at least one GPU backend configured.
