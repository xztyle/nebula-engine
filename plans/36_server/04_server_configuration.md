# Server Configuration

## Problem

The dedicated server needs its own configuration, separate from the client's `config.ron`. Server-specific settings do not belong in the client config and vice versa. Without server configuration:

- **Bind address is hardcoded** — Every deployment requires recompilation to change the listen address or port. Production servers, staging environments, and local development all need different bind addresses.
- **Max players cannot be tuned** — Different server hardware can handle different player counts. A home server on a Raspberry Pi should cap at 4 players; a dedicated machine with 64 cores should allow 256. This must be configurable without code changes.
- **World seeds are not reproducible** — Without a configurable seed, every server restart generates a different world. Testing terrain generation, reproducing bugs, and running tournament servers with identical maps all require a fixed seed.
- **Render distance is conflated** — The server does not render, but it must know the chunk generation radius around each player. Using the client's render distance setting is wrong because the server must generate terrain ahead of what the client currently sees. This needs its own setting.
- **No admin password** — Without authentication, anyone who connects to the admin CLI or RCON port can execute admin commands. The server needs a configurable password.
- **Save directory is not configurable** — Different servers running on the same machine need separate save directories to avoid data corruption.

The server config uses the same RON format as the client config (see `01_setup/07_configuration_system.md`) for consistency, but is stored in a separate file (`server_config.ron`) with server-specific fields.

## Solution

### Configuration Struct

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ServerConfig {
    /// IP address to bind the TCP listener to.
    /// Use "0.0.0.0" to listen on all interfaces.
    pub bind_address: String,

    /// TCP port to listen on.
    pub port: u16,

    /// Maximum number of simultaneous player connections.
    pub max_players: u32,

    /// World generation seed. Determines terrain, biomes, ore distribution.
    /// Use the same seed across servers for identical worlds.
    pub world_seed: u64,

    /// Chunk generation radius around each player (in chunks).
    /// This determines how far ahead the server generates terrain
    /// beyond what the player currently sees. Should be >= client
    /// render distance + a buffer (typically +2 to +4 chunks).
    pub generation_radius: u32,

    /// Directory for world saves, player data, and ban lists.
    /// Relative paths are resolved from the server binary's working directory.
    pub save_directory: PathBuf,

    /// Server simulation tick rate in Hz.
    /// Must match the client's FixedUpdate rate for deterministic simulation.
    /// Default: 60. Do not change unless you know what you are doing.
    pub tick_rate: u32,

    /// Password required for admin CLI commands over RCON.
    /// Leave empty to disable password authentication (local-only servers).
    pub admin_password: String,

    /// Server name displayed to players in the server browser / MOTD.
    pub server_name: String,

    /// Message of the day displayed to players on connect.
    pub motd: String,

    /// Whether to enable automatic world saving at regular intervals.
    pub auto_save: bool,

    /// Interval between automatic world saves, in seconds.
    pub auto_save_interval_secs: u64,

    /// Maximum view distance a client is allowed to request (in chunks).
    /// Prevents clients from requesting absurd distances that overload the server.
    pub max_view_distance: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0".to_string(),
            port: 7777,
            max_players: 32,
            world_seed: 0,
            generation_radius: 20,
            save_directory: PathBuf::from("world"),
            tick_rate: 60,
            admin_password: String::new(),
            server_name: "Nebula Server".to_string(),
            motd: "Welcome to Nebula Engine!".to_string(),
            auto_save: true,
            auto_save_interval_secs: 300, // 5 minutes
            max_view_distance: 32,
        }
    }
}
```

### Generated RON File

A default `server_config.ron` looks like:

```ron
// Nebula Engine Dedicated Server Configuration
(
    bind_address: "0.0.0.0",
    port: 7777,
    max_players: 32,
    world_seed: 0,
    generation_radius: 20,
    save_directory: "world",
    tick_rate: 60,
    admin_password: "",
    server_name: "Nebula Server",
    motd: "Welcome to Nebula Engine!",
    auto_save: true,
    auto_save_interval_secs: 300,
    max_view_distance: 32,
)
```

### Loading with Defaults for Missing Fields

The `#[serde(default)]` attribute on the struct ensures that any missing field in the RON file falls back to its `Default` value. This provides forward compatibility — when new fields are added in future versions, existing config files continue to load without error.

```rust
impl ServerConfig {
    /// Load config from the specified path.
    /// If the file does not exist, create it with defaults.
    pub fn load_or_default(path: &std::path::Path) -> Self {
        if path.exists() {
            match std::fs::read_to_string(path) {
                Ok(contents) => {
                    match ron::from_str::<ServerConfig>(&contents) {
                        Ok(config) => {
                            tracing::info!("Loaded server config from {}", path.display());
                            return config;
                        }
                        Err(e) => {
                            tracing::error!(
                                "Failed to parse {}: {e}. Using defaults.",
                                path.display()
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to read {}: {e}. Using defaults.",
                        path.display()
                    );
                }
            }
        } else {
            tracing::info!(
                "No config file at {}, creating default",
                path.display()
            );
        }

        let config = ServerConfig::default();
        config.save(path);
        config
    }

    /// Save config to disk in pretty RON format.
    pub fn save(&self, path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let pretty = ron::ser::PrettyConfig::new()
            .depth_limit(2)
            .separate_tuple_members(true);

        match ron::ser::to_string_pretty(self, pretty) {
            Ok(serialized) => {
                if let Err(e) = std::fs::write(path, serialized) {
                    tracing::error!("Failed to write config: {e}");
                }
            }
            Err(e) => {
                tracing::error!("Failed to serialize config: {e}");
            }
        }
    }
}
```

### CLI Overrides

Command-line arguments (parsed with `clap` in the server binary, see story 01) take precedence over file values. The override pattern matches the client's approach:

```rust
impl ServerConfig {
    pub fn apply_cli_overrides(
        &mut self,
        bind: Option<&str>,
        port: Option<u16>,
        max_players: Option<u32>,
        seed: Option<u64>,
        tick_rate: Option<u32>,
    ) {
        if let Some(b) = bind {
            self.bind_address = b.to_string();
        }
        if let Some(p) = port {
            self.port = p;
        }
        if let Some(m) = max_players {
            self.max_players = m;
        }
        if let Some(s) = seed {
            self.world_seed = s;
        }
        if let Some(t) = tick_rate {
            self.tick_rate = t;
        }
    }
}
```

### Validation

Config values are validated after loading and after applying CLI overrides. Invalid values produce a clear error and fall back to defaults or terminate:

```rust
#[derive(Debug)]
pub enum ConfigValidationError {
    PortOutOfRange(u16),
    MaxPlayersZero,
    TickRateZero,
    TickRateTooHigh(u32),
    GenerationRadiusZero,
    SaveDirectoryInvalid(String),
}

impl ServerConfig {
    pub fn validate(&self) -> Result<(), Vec<ConfigValidationError>> {
        let mut errors = Vec::new();

        // Port must be in valid range (0 is technically valid but useless)
        if self.port == 0 {
            errors.push(ConfigValidationError::PortOutOfRange(self.port));
        }

        // Must allow at least 1 player
        if self.max_players == 0 {
            errors.push(ConfigValidationError::MaxPlayersZero);
        }

        // Tick rate must be positive and reasonable
        if self.tick_rate == 0 {
            errors.push(ConfigValidationError::TickRateZero);
        }
        if self.tick_rate > 240 {
            errors.push(ConfigValidationError::TickRateTooHigh(self.tick_rate));
        }

        // Generation radius must be positive
        if self.generation_radius == 0 {
            errors.push(ConfigValidationError::GenerationRadiusZero);
        }

        // Save directory path must not be empty
        if self.save_directory.as_os_str().is_empty() {
            errors.push(ConfigValidationError::SaveDirectoryInvalid(
                "empty path".to_string(),
            ));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}
```

### Startup Flow

```
1. Parse CLI args (clap)
2. Load server_config.ron (or create default)
3. Apply CLI overrides
4. Validate config
5. Log final config values at info level
6. Pass config to server tick loop and networking
```

### Runtime Access

The config is inserted into the ECS world as a resource so any system can read it:

```rust
world.insert_resource(config.clone());
```

Systems query it with `Res<ServerConfig>`. The config is immutable at runtime — changes require a server restart. This avoids the complexity of hot-reloading server settings while the simulation is running.

## Outcome

A `config.rs` module in `crates/nebula-server/src/` exporting `ServerConfig` and `ConfigValidationError`. The config loads from `server_config.ron` in RON format with `#[serde(default)]` for forward compatibility. CLI arguments override file values. All values are validated with clear error messages. The config is inserted into the ECS world as a resource. Missing config files are created with sensible defaults on first run. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

`server.ron` configures: port, max players, world seed, render distance, save interval, and admin password. The server reads the configuration at startup.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `serde` | `1.0` (features: `derive`) | `Serialize` and `Deserialize` derives for the config struct |
| `ron` | `0.9` | RON format parsing and pretty-printing, consistent with client config |
| `clap` | `4` (features: `derive`) | CLI argument parsing for config overrides |
| `tracing` | `0.1` | Logging config load/save events and validation errors |
| `bevy_ecs` | `0.18` | Insert config as an ECS `Resource` for system access |

## Unit Tests

- **`test_config_loads_from_ron`** — Write a RON string with non-default values (`port: 8888`, `max_players: 64`, `world_seed: 42`) to a temporary file. Load with `ServerConfig::load_or_default`. Assert `port == 8888`, `max_players == 64`, `world_seed == 42`.

- **`test_missing_values_use_defaults`** — Write a RON string with only `(port: 9999)` to a temporary file. Load the config. Assert `port == 9999` and all other fields equal `ServerConfig::default()` values (`max_players == 32`, `bind_address == "0.0.0.0"`, etc.).

- **`test_cli_overrides_file_values`** — Load a config with `port: 7777` from file. Call `apply_cli_overrides` with `port: Some(8080)`. Assert `config.port == 8080`. Assert other fields remain unchanged.

- **`test_invalid_port_zero_produces_error`** — Create a config with `port: 0`. Call `validate()`. Assert the result is `Err` containing `ConfigValidationError::PortOutOfRange(0)`.

- **`test_invalid_max_players_zero`** — Create a config with `max_players: 0`. Call `validate()`. Assert the result is `Err` containing `ConfigValidationError::MaxPlayersZero`.

- **`test_invalid_tick_rate_zero`** — Create a config with `tick_rate: 0`. Call `validate()`. Assert `Err` containing `ConfigValidationError::TickRateZero`.

- **`test_invalid_tick_rate_too_high`** — Create a config with `tick_rate: 1000`. Call `validate()`. Assert `Err` containing `ConfigValidationError::TickRateTooHigh(1000)`.

- **`test_valid_config_passes_validation`** — Call `validate()` on `ServerConfig::default()`. Assert `Ok(())`.

- **`test_config_roundtrip`** — Create a `ServerConfig` with non-default values. Serialize to RON, deserialize back. Assert equality via `PartialEq`.

- **`test_config_accessible_as_ecs_resource`** — Create a `bevy_ecs::world::World`, insert a `ServerConfig` as a resource. Query it with `world.resource::<ServerConfig>()`. Assert the returned config matches the inserted one.

- **`test_nonexistent_file_creates_default`** — Call `load_or_default` with a path that does not exist. Assert the returned config equals `ServerConfig::default()`. Assert the file now exists on disk.

- **`test_malformed_ron_falls_back_to_default`** — Write `"{{not valid ron}}"` to a file. Call `load_or_default`. Assert the returned config equals `ServerConfig::default()` (fallback on parse error, not a panic).
