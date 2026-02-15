# Configuration System

## Problem

The engine needs runtime-configurable settings that persist across sessions and are easy to edit by hand. Without a configuration system:

- **Window size** is hardcoded and cannot be changed without recompiling.
- **Render distance** requires a code change to tune, which is unacceptable for both development iteration and end-user customization.
- **Keybindings** are fixed, alienating players who use non-QWERTY layouts or have accessibility needs.
- **Server address** for multiplayer is hardcoded, making testing between local and remote servers painful.
- **Developer settings** (debug overlays, wireframe mode, log levels) require recompilation to toggle.

The configuration format must be human-readable (not binary), support comments (for self-documentation), and map naturally to Rust types. JSON lacks comments and is verbose. TOML is an option but nests poorly for deeply structured configs. RON (Rusty Object Notation) mirrors Rust syntax, supports comments, and handles enums and nested structs naturally, making it the ideal choice for a Rust engine.

## Solution

### Configuration Struct Hierarchy

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub window: WindowConfig,
    pub render: RenderConfig,
    pub input: InputConfig,
    pub network: NetworkConfig,
    pub audio: AudioConfig,
    pub debug: DebugConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WindowConfig {
    /// Window width in logical pixels
    pub width: u32,
    /// Window height in logical pixels
    pub height: u32,
    /// Start in fullscreen mode
    pub fullscreen: bool,
    /// Enable vsync (PresentMode::Fifo)
    pub vsync: bool,
    /// Window title
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RenderConfig {
    /// Render distance in chunks
    pub render_distance: u32,
    /// LOD bias (higher = more aggressive LOD reduction)
    pub lod_bias: f32,
    /// Maximum shadow cascade distance
    pub shadow_distance: f32,
    /// Enable ambient occlusion
    pub ambient_occlusion: bool,
    /// MSAA sample count (1, 2, 4)
    pub msaa_samples: u32,
    /// Target frame rate (0 = unlimited / vsync)
    pub target_fps: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct InputConfig {
    /// Mouse sensitivity multiplier
    pub mouse_sensitivity: f32,
    /// Invert Y axis for camera
    pub invert_y: bool,
    /// Keybinding overrides (action name -> key name)
    pub keybindings: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NetworkConfig {
    /// Server address for multiplayer
    pub server_address: String,
    /// Server port
    pub server_port: u16,
    /// Client timeout in seconds
    pub timeout_seconds: u32,
    /// Maximum number of players (server only)
    pub max_players: u32,
    /// Tick rate for network updates (Hz)
    pub net_tick_rate: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AudioConfig {
    /// Master volume (0.0 - 1.0)
    pub master_volume: f32,
    /// Music volume (0.0 - 1.0)
    pub music_volume: f32,
    /// Sound effects volume (0.0 - 1.0)
    pub sfx_volume: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DebugConfig {
    /// Show FPS overlay
    pub show_fps: bool,
    /// Show chunk boundaries
    pub show_chunk_boundaries: bool,
    /// Show physics collider wireframes
    pub show_colliders: bool,
    /// Enable wireframe rendering
    pub wireframe_mode: bool,
    /// Log level override (e.g., "debug", "info", "warn")
    pub log_level: String,
}
```

### Default Implementations

Every config section implements `Default` with sensible values:

```rust
impl Default for Config {
    fn default() -> Self {
        Self {
            window: WindowConfig::default(),
            render: RenderConfig::default(),
            input: InputConfig::default(),
            network: NetworkConfig::default(),
            audio: AudioConfig::default(),
            debug: DebugConfig::default(),
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            fullscreen: false,
            vsync: true,
            title: "Nebula Engine".to_string(),
        }
    }
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            render_distance: 16,
            lod_bias: 1.0,
            shadow_distance: 256.0,
            ambient_occlusion: true,
            msaa_samples: 4,
            target_fps: 0,
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            server_address: "127.0.0.1".to_string(),
            server_port: 7777,
            timeout_seconds: 30,
            max_players: 32,
            net_tick_rate: 20,
        }
    }
}

// ... similar for InputConfig, AudioConfig, DebugConfig
```

### Loading and Saving

```rust
impl Config {
    /// Load config from the platform config directory.
    /// If the file does not exist, create it with defaults.
    pub fn load_or_create(config_dir: &Path) -> Result<Self, ConfigError> {
        let config_path = config_dir.join("config.ron");

        if config_path.exists() {
            let contents = std::fs::read_to_string(&config_path)
                .map_err(ConfigError::ReadError)?;
            let config: Config = ron::from_str(&contents)
                .map_err(ConfigError::ParseError)?;
            log::info!("Loaded config from {}", config_path.display());
            Ok(config)
        } else {
            let config = Config::default();
            config.save(config_dir)?;
            log::info!(
                "Created default config at {}",
                config_path.display()
            );
            Ok(config)
        }
    }

    /// Save config to the platform config directory.
    pub fn save(&self, config_dir: &Path) -> Result<(), ConfigError> {
        std::fs::create_dir_all(config_dir)
            .map_err(ConfigError::WriteError)?;

        let config_path = config_dir.join("config.ron");
        let pretty = ron::ser::PrettyConfig::new()
            .depth_limit(3)
            .separate_tuple_members(true)
            .enumerate_arrays(false);

        let serialized = ron::ser::to_string_pretty(self, pretty)
            .map_err(ConfigError::SerializeError)?;

        std::fs::write(&config_path, serialized)
            .map_err(ConfigError::WriteError)?;

        Ok(())
    }

    /// Hot-reload the config from disk.
    /// Returns the new config if the file has changed, None otherwise.
    pub fn reload(&self, config_dir: &Path) -> Result<Option<Self>, ConfigError> {
        let config_path = config_dir.join("config.ron");
        let contents = std::fs::read_to_string(&config_path)
            .map_err(ConfigError::ReadError)?;
        let new_config: Config = ron::from_str(&contents)
            .map_err(ConfigError::ParseError)?;

        if &new_config != self {
            log::info!("Config reloaded with changes");
            Ok(Some(new_config))
        } else {
            Ok(None)
        }
    }
}
```

### Generated RON File

A default `config.ron` looks like this:

```ron
// Nebula Engine Configuration
(
    window: (
        width: 1280,
        height: 720,
        fullscreen: false,
        vsync: true,
        title: "Nebula Engine",
    ),
    render: (
        render_distance: 16,
        lod_bias: 1.0,
        shadow_distance: 256.0,
        ambient_occlusion: true,
        msaa_samples: 4,
        target_fps: 0,
    ),
    input: (
        mouse_sensitivity: 1.0,
        invert_y: false,
        keybindings: {},
    ),
    network: (
        server_address: "127.0.0.1",
        server_port: 7777,
        timeout_seconds: 30,
        max_players: 32,
        net_tick_rate: 20,
    ),
    audio: (
        master_volume: 1.0,
        music_volume: 0.7,
        sfx_volume: 1.0,
    ),
    debug: (
        show_fps: false,
        show_chunk_boundaries: false,
        show_colliders: false,
        wireframe_mode: false,
        log_level: "info",
    ),
)
```

### CLI Overrides with Clap

Command-line arguments take precedence over file values:

```rust
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "nebula", about = "Nebula Engine")]
pub struct CliArgs {
    /// Window width
    #[arg(long)]
    pub width: Option<u32>,

    /// Window height
    #[arg(long)]
    pub height: Option<u32>,

    /// Start in fullscreen
    #[arg(long)]
    pub fullscreen: Option<bool>,

    /// Server address
    #[arg(long)]
    pub server: Option<String>,

    /// Server port
    #[arg(long)]
    pub port: Option<u16>,

    /// Render distance in chunks
    #[arg(long)]
    pub render_distance: Option<u32>,

    /// Log level (error, warn, info, debug, trace)
    #[arg(long)]
    pub log_level: Option<String>,

    /// Path to config file (overrides default location)
    #[arg(long)]
    pub config: Option<PathBuf>,
}

impl Config {
    /// Apply CLI overrides to a loaded config.
    pub fn apply_cli_overrides(&mut self, args: &CliArgs) {
        if let Some(w) = args.width {
            self.window.width = w;
        }
        if let Some(h) = args.height {
            self.window.height = h;
        }
        if let Some(fs) = args.fullscreen {
            self.window.fullscreen = fs;
        }
        if let Some(ref addr) = args.server {
            self.network.server_address = addr.clone();
        }
        if let Some(port) = args.port {
            self.network.server_port = port;
        }
        if let Some(rd) = args.render_distance {
            self.render.render_distance = rd;
        }
        if let Some(ref level) = args.log_level {
            self.debug.log_level = level.clone();
        }
    }
}
```

### Startup Flow

```
1. Parse CLI args (clap)
2. Resolve config directory (dirs crate, or --config override)
3. Load config.ron (or create default)
4. Apply CLI overrides
5. Initialize engine with final Config
```

### Hot-Reload in Development

During development, a file watcher (using `notify` crate, added later) can watch `config.ron` and call `Config::reload()` on change. For now, hot-reload is manual: press a key (e.g., F5) to trigger a reload. The config reload method returns `Some(new_config)` only if the file actually changed, avoiding unnecessary reinitialization.

## Outcome

A `config.ron` file is created on first run with sensible defaults. The file is human-readable with RON syntax that mirrors Rust types. Editing it and restarting the engine applies the changes. CLI flags override file values for quick testing (e.g., `--render_distance 4` for performance testing). In development builds, hot-reloading allows tweaking settings without restarting the engine. Missing fields in the config file default to sensible values (forward compatibility when adding new settings).

## Demo Integration

**Demo crate:** `nebula-demo`

Window size, title, and the clear color are configurable via `config.ron` without recompiling. A developer can change the demo's appearance by editing a text file.

## Crates & Dependencies

- **`ron = "0.12"`** — RON (Rusty Object Notation) serialization/deserialization. Chosen over JSON (no comments, verbose) and TOML (poor nesting support) because RON mirrors Rust syntax, supports comments, and handles enums naturally.
- **`serde = { version = "1", features = ["derive"] }`** — The standard Rust serialization framework. The `derive` feature enables `#[derive(Serialize, Deserialize)]` on config structs.
- **`clap = { version = "4", features = ["derive"] }`** — Command-line argument parsing with derive macros. Provides `--help` output, type validation, and optional arguments out of the box.
- **`dirs = "6"`** — Cross-platform config directory resolution (already a dependency from `03_cross_platform_build_validation.md`).

## Unit Tests

- **`test_default_config_serializes`** — Serialize `Config::default()` to a RON string and verify the result is non-empty and contains expected substrings (e.g., `"width: 1280"`, `"server_port: 7777"`).

- **`test_config_roundtrip`** — Serialize `Config::default()` to RON, then deserialize back, and verify the result equals the original using `PartialEq`. This validates that no information is lost during serialization.

- **`test_missing_field_uses_default`** — Deserialize a RON string that is missing the `audio` section entirely. Verify that the resulting `Config` has `AudioConfig::default()` values. This tests `#[serde(default)]` behavior and ensures forward compatibility.

- **`test_extra_field_ignored`** — Deserialize a RON string that contains an unknown field (e.g., `future_setting: true`). Verify that deserialization succeeds without error. This tests backward compatibility when config files from newer versions are loaded by older engine builds.

- **`test_cli_override`** — Create a `Config::default()` and a `CliArgs` with `width: Some(1920)` and `server: Some("192.168.1.1".to_string())`. Call `apply_cli_overrides()` and verify that `config.window.width` is 1920 and `config.network.server_address` is `"192.168.1.1"`. Verify that non-overridden fields retain their defaults.

- **`test_cli_no_override`** — Create a `Config::default()` and a `CliArgs` with all `None` values. Call `apply_cli_overrides()` and verify the config is unchanged (equals the original default).

- **`test_save_and_load`** — Create a `Config` with non-default values, save it to a temporary directory, load it back, and verify equality. This tests the full file I/O path.

- **`test_reload_detects_changes`** — Save a config, modify a field, save again, and call `reload()`. Verify it returns `Some(new_config)` with the changed field.

- **`test_reload_no_changes`** — Save a config and immediately call `reload()` without modifying the file. Verify it returns `None`.

- **`test_invalid_ron_produces_error`** — Attempt to deserialize an invalid RON string (e.g., `"{{not valid}}"`) and verify a `ConfigError::ParseError` is returned with a meaningful message.

- **`test_ron_comments_preserved`** — Verify that RON files with comments (lines starting with `//`) parse correctly. This ensures users can annotate their config files.
