//! Command-line argument parsing for Nebula Engine.

use std::path::PathBuf;

use clap::Parser;

use crate::Config;

/// Nebula Engine command-line arguments.
///
/// CLI values override settings loaded from `config.ron`.
#[derive(Parser, Debug)]
#[command(name = "nebula", about = "Nebula Engine")]
pub struct CliArgs {
    /// Window width.
    #[arg(long)]
    pub width: Option<u32>,

    /// Window height.
    #[arg(long)]
    pub height: Option<u32>,

    /// Start in fullscreen.
    #[arg(long)]
    pub fullscreen: Option<bool>,

    /// Server address.
    #[arg(long)]
    pub server: Option<String>,

    /// Server port.
    #[arg(long)]
    pub port: Option<u16>,

    /// Render distance in chunks.
    #[arg(long)]
    pub render_distance: Option<u32>,

    /// Log level (error, warn, info, debug, trace).
    #[arg(long)]
    pub log_level: Option<String>,

    /// Path to config directory (overrides default location).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_override() {
        let mut config = Config::default();
        let args = CliArgs {
            width: Some(1920),
            height: None,
            fullscreen: None,
            server: Some("192.168.1.1".to_string()),
            port: None,
            render_distance: None,
            log_level: None,
            config: None,
        };
        config.apply_cli_overrides(&args);
        assert_eq!(config.window.width, 1920);
        assert_eq!(config.network.server_address, "192.168.1.1");
        // Non-overridden fields retain defaults
        assert_eq!(config.window.height, 720);
        assert_eq!(config.network.server_port, 7777);
    }

    #[test]
    fn test_cli_no_override() {
        let original = Config::default();
        let mut config = Config::default();
        let args = CliArgs {
            width: None,
            height: None,
            fullscreen: None,
            server: None,
            port: None,
            render_distance: None,
            log_level: None,
            config: None,
        };
        config.apply_cli_overrides(&args);
        assert_eq!(config, original);
    }
}
