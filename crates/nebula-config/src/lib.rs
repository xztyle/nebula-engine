//! Configuration system for Nebula Engine.
//!
//! Provides runtime-configurable settings that persist to disk as RON files.
//! Supports CLI overrides via clap, hot-reload detection, and forward/backward
//! compatible serialization.

mod cli;
mod config;
mod error;

pub use cli::CliArgs;
pub use config::{
    AudioConfig, Config, DebugConfig, InputConfig, NetworkConfig, PlanetConfig, RenderConfig,
    WindowConfig,
};
pub use error::ConfigError;
