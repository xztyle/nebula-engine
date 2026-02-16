//! Configuration structs with sensible defaults and RON persistence.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;

/// Top-level engine configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Window settings.
    pub window: WindowConfig,
    /// Rendering settings.
    pub render: RenderConfig,
    /// Input settings.
    pub input: InputConfig,
    /// Network/multiplayer settings.
    pub network: NetworkConfig,
    /// Audio settings.
    pub audio: AudioConfig,
    /// Debug/development settings.
    pub debug: DebugConfig,
    /// Planet settings.
    pub planet: PlanetConfig,
}

/// Window configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WindowConfig {
    /// Window width in logical pixels.
    pub width: u32,
    /// Window height in logical pixels.
    pub height: u32,
    /// Start in fullscreen mode.
    pub fullscreen: bool,
    /// Enable vsync (PresentMode::Fifo).
    pub vsync: bool,
    /// Window title.
    pub title: String,
}

/// Rendering configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RenderConfig {
    /// Render distance in chunks.
    pub render_distance: u32,
    /// LOD bias (higher = more aggressive LOD reduction).
    pub lod_bias: f32,
    /// Maximum shadow cascade distance.
    pub shadow_distance: f32,
    /// Enable ambient occlusion.
    pub ambient_occlusion: bool,
    /// MSAA sample count (1, 2, 4).
    pub msaa_samples: u32,
    /// Target frame rate (0 = unlimited / vsync).
    pub target_fps: u32,
}

/// Input configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct InputConfig {
    /// Mouse sensitivity multiplier.
    pub mouse_sensitivity: f32,
    /// Invert Y axis for camera.
    pub invert_y: bool,
    /// Keybinding overrides (action name -> key name).
    pub keybindings: HashMap<String, String>,
}

/// Network/multiplayer configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NetworkConfig {
    /// Server address for multiplayer.
    pub server_address: String,
    /// Server port.
    pub server_port: u16,
    /// Client timeout in seconds.
    pub timeout_seconds: u32,
    /// Maximum number of players (server only).
    pub max_players: u32,
    /// Tick rate for network updates (Hz).
    pub net_tick_rate: u32,
}

/// Audio configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AudioConfig {
    /// Master volume (0.0 - 1.0).
    pub master_volume: f32,
    /// Music volume (0.0 - 1.0).
    pub music_volume: f32,
    /// Sound effects volume (0.0 - 1.0).
    pub sfx_volume: f32,
}

/// Debug/development configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DebugConfig {
    /// Show FPS overlay.
    pub show_fps: bool,
    /// Show chunk boundaries.
    pub show_chunk_boundaries: bool,
    /// Show physics collider wireframes.
    pub show_colliders: bool,
    /// Enable wireframe rendering.
    pub wireframe_mode: bool,
    /// Log level override (e.g., "debug", "info", "warn").
    pub log_level: String,
}

/// Planet configuration for the game world.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PlanetConfig {
    /// Planet radius in meters.
    pub radius_m: f64,
    /// Camera starting altitude above the surface in meters.
    pub start_altitude_m: f64,
    /// Enable free-fly camera (WASD + mouse look) instead of orbiting demo camera.
    pub free_fly_camera: bool,
    /// Free-fly camera speed in meters per second.
    pub camera_speed_m_s: f64,
}

impl Default for PlanetConfig {
    fn default() -> Self {
        Self {
            radius_m: 200.0,
            start_altitude_m: 600.0,
            free_fly_camera: false,
            camera_speed_m_s: 1000.0,
        }
    }
}

// --- Default implementations ---

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

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            mouse_sensitivity: 1.0,
            invert_y: false,
            keybindings: HashMap::new(),
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

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            master_volume: 1.0,
            music_volume: 0.7,
            sfx_volume: 1.0,
        }
    }
}

impl Default for DebugConfig {
    fn default() -> Self {
        Self {
            show_fps: false,
            show_chunk_boundaries: false,
            show_colliders: false,
            wireframe_mode: false,
            log_level: "info".to_string(),
        }
    }
}

// --- Load / Save / Reload ---

impl Config {
    /// Load config from the given directory, or create a default config file.
    pub fn load_or_create(config_dir: &Path) -> Result<Self, ConfigError> {
        let config_path = config_dir.join("config.ron");

        if config_path.exists() {
            let contents = std::fs::read_to_string(&config_path).map_err(ConfigError::ReadError)?;
            let config: Config = ron::from_str(&contents).map_err(ConfigError::ParseError)?;
            log::info!("Loaded config from {}", config_path.display());
            Ok(config)
        } else {
            let config = Config::default();
            config.save(config_dir)?;
            log::info!("Created default config at {}", config_path.display());
            Ok(config)
        }
    }

    /// Save config to the given directory as `config.ron`.
    pub fn save(&self, config_dir: &Path) -> Result<(), ConfigError> {
        std::fs::create_dir_all(config_dir).map_err(ConfigError::WriteError)?;

        let config_path = config_dir.join("config.ron");
        let pretty = ron::ser::PrettyConfig::new()
            .depth_limit(3)
            .separate_tuple_members(true)
            .enumerate_arrays(false);

        let serialized =
            ron::ser::to_string_pretty(self, pretty).map_err(ConfigError::SerializeError)?;

        std::fs::write(&config_path, serialized).map_err(ConfigError::WriteError)?;
        Ok(())
    }

    /// Hot-reload: returns `Some(new_config)` if the file changed, `None` otherwise.
    pub fn reload(&self, config_dir: &Path) -> Result<Option<Self>, ConfigError> {
        let config_path = config_dir.join("config.ron");
        let contents = std::fs::read_to_string(&config_path).map_err(ConfigError::ReadError)?;
        let new_config: Config = ron::from_str(&contents).map_err(ConfigError::ParseError)?;

        if &new_config != self {
            log::info!("Config reloaded with changes");
            Ok(Some(new_config))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_serializes() {
        let config = Config::default();
        let ron_str =
            ron::ser::to_string_pretty(&config, ron::ser::PrettyConfig::new().depth_limit(3))
                .unwrap();
        assert!(!ron_str.is_empty());
        assert!(ron_str.contains("width: 1280"));
        assert!(ron_str.contains("server_port: 7777"));
    }

    #[test]
    fn test_config_roundtrip() {
        let config = Config::default();
        let ron_str = ron::to_string(&config).unwrap();
        let deserialized: Config = ron::from_str(&ron_str).unwrap();
        assert_eq!(config, deserialized);
    }

    #[test]
    fn test_missing_field_uses_default() {
        // Config missing the `audio` section entirely
        let ron_str = "(window: (), render: (), input: (), network: (), debug: ())";
        let config: Config = ron::from_str(ron_str).unwrap();
        assert_eq!(config.audio, AudioConfig::default());
    }

    #[test]
    fn test_extra_field_ignored() {
        let ron_str = "(future_setting: true)";
        // RON with #[serde(default)] and deny_unknown_fields not set should accept this
        let result: Result<Config, _> = ron::from_str(ron_str);
        assert!(result.is_ok());
    }

    #[test]
    fn test_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.window.width = 1920;
        config.window.height = 1080;
        config.network.server_address = "10.0.0.1".to_string();

        config.save(dir.path()).unwrap();
        let loaded = Config::load_or_create(dir.path()).unwrap();
        assert_eq!(config, loaded);
    }

    #[test]
    fn test_reload_detects_changes() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::default();
        config.save(dir.path()).unwrap();

        let mut modified = config.clone();
        modified.window.width = 1920;
        modified.save(dir.path()).unwrap();

        let result = config.reload(dir.path()).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().window.width, 1920);
    }

    #[test]
    fn test_reload_no_changes() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::default();
        config.save(dir.path()).unwrap();

        let result = config.reload(dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_invalid_ron_produces_error() {
        let result: Result<Config, _> = ron::from_str("{{not valid}}");
        assert!(result.is_err());
    }

    #[test]
    fn test_ron_comments_preserved() {
        let ron_str = "// This is a comment\n(\n  // Another comment\n)";
        let config: Config = ron::from_str(ron_str).unwrap();
        assert_eq!(config, Config::default());
    }
}
