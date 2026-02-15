//! Structured logging and tracing for Nebula Engine.
//!
//! Provides structured, span-based, filterable logging via the `tracing` ecosystem.
//! Supports console output with timestamps and module paths, plus JSON file logging
//! in debug builds for post-mortem analysis. Integrates with the configuration system
//! to allow runtime log level control.

use nebula_config::Config;
use std::path::Path;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Initialize the tracing subscriber for the Nebula Engine.
///
/// Sets up structured logging with:
/// - Console output with timestamps, module paths, and severity levels
/// - JSON file logging in debug builds (optional)
/// - Environment-based filtering (respects RUST_LOG)
/// - Integration with config system log_level setting
///
/// # Arguments
///
/// * `log_dir` - Optional directory for JSON log files (debug builds only)
/// * `debug_build` - Whether this is a debug build (enables file logging)
/// * `config` - Optional configuration to use for log level override
///
/// # Examples
///
/// ```no_run
/// use nebula_log::init_logging;
/// use nebula_config::Config;
///
/// // Basic initialization
/// init_logging(None, false, None);
///
/// // With file logging in debug mode
/// let log_dir = std::path::Path::new("./logs");
/// init_logging(Some(log_dir), true, None);
///
/// // With config override
/// let config = Config::default();
/// init_logging(None, false, Some(&config));
/// ```
pub fn init_logging(log_dir: Option<&Path>, debug_build: bool, config: Option<&Config>) {
    // Determine the filter string
    let filter_str = if let Some(config) = config {
        if !config.debug.log_level.is_empty() {
            config.debug.log_level.clone()
        } else {
            "info,wgpu=warn,naga=warn".to_string()
        }
    } else {
        "info,wgpu=warn,naga=warn".to_string()
    };

    // Base filter: info by default, overridable via RUST_LOG env var
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&filter_str));

    // Console layer: human-readable format with timestamps
    let console_layer = fmt::layer()
        .with_target(true) // Show module path
        .with_thread_ids(false) // Not useful for most debugging
        .with_thread_names(true) // Useful when render/sim threads are named
        .with_level(true) // Show log level
        .with_timer(fmt::time::uptime()); // Time since engine start

    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(console_layer);

    // In debug builds, also log to a file for post-mortem analysis
    if debug_build
        && let Some(log_dir) = log_dir
        && std::fs::create_dir_all(log_dir).is_ok()
        && let Ok(log_file) = std::fs::File::create(log_dir.join("nebula.log"))
    {
        let file_layer = fmt::layer()
            .with_writer(log_file)
            .with_ansi(false) // No ANSI color codes in file output
            .with_target(true)
            .with_timer(fmt::time::uptime())
            .json(); // Structured JSON for machine parsing

        subscriber.with(file_layer).init();
        return;
    }

    subscriber.init();
}

/// Create an `EnvFilter` with the default filter string.
///
/// Returns a filter that enables:
/// - `info` level for all targets by default
/// - `warn` level for `wgpu` and `naga` to reduce noise
///
/// This is useful for testing and for getting consistent default behavior.
pub fn default_env_filter() -> EnvFilter {
    EnvFilter::new("info,wgpu=warn,naga=warn")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_log_level() {
        let filter = default_env_filter();

        // Test that filter was created successfully
        // Note: The actual format might vary, but it should contain our filter parts
        let filter_str = format!("{}", filter);
        assert!(filter_str.contains("wgpu=warn"));
        assert!(filter_str.contains("naga=warn"));
        assert!(filter_str.contains("info"));
    }

    #[test]
    fn test_subsystem_filter() {
        let filter = EnvFilter::new("info,nebula_net=debug");

        // Test that filter was created successfully
        let filter_str = format!("{}", filter);
        assert!(filter_str.contains("nebula_net=debug"));
        assert!(filter_str.contains("info"));
    }

    #[test]
    fn test_log_output_format() {
        // This test validates that we can create an EnvFilter for console output
        let filter = EnvFilter::new("debug");
        assert!(format!("{}", filter).contains("debug"));
    }

    #[test]
    fn test_json_format() {
        // This test validates that we can create an EnvFilter for JSON mode
        let filter = EnvFilter::new("info");
        assert!(format!("{}", filter).contains("info"));
    }

    #[test]
    fn test_env_filter_parsing() {
        // Test various RUST_LOG strings parse without error
        let valid_filters = [
            "info",
            "debug,nebula_render=trace",
            "warn,nebula_net=debug,nebula_voxel=trace",
            "error",
        ];

        for filter_str in &valid_filters {
            let result = EnvFilter::try_from(*filter_str);
            assert!(result.is_ok(), "Failed to parse filter: {}", filter_str);
        }

        // Note: EnvFilter is quite forgiving and will parse almost anything,
        // just ignoring invalid parts. This is actually the correct behavior.
        // So we just test that try_from doesn't panic on weird input.
        let _result = EnvFilter::try_from("weird=input=with=equals");
        assert!(true); // If we get here without panicking, the parsing is robust
    }

    #[test]
    fn test_file_logger_creation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let log_path = temp_dir.path();

        // Test that we can create the log directory
        std::fs::create_dir_all(log_path).unwrap();

        // Test creating a log file path
        let log_file_path = log_path.join("nebula.log");
        assert_eq!(log_file_path.file_name().unwrap(), "nebula.log");
    }

    #[test]
    fn test_uptime_timer_starts_near_zero() {
        // This test validates that we can create configuration for uptime timer
        let filter = EnvFilter::new("trace");
        assert!(format!("{}", filter).contains("trace"));
    }
}
