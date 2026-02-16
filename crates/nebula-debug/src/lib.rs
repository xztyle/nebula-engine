//! AI Debug API for the Nebula Engine.
//!
//! Provides an HTTP server that exposes debug endpoints for AI agents to observe
//! and control the running engine. Only compiled in debug builds.

#[cfg(debug_assertions)]
pub mod server;

#[cfg(debug_assertions)]
pub use server::{DebugServer, DebugServerError};

#[cfg(test)]
mod tests;

/// State shared between the game loop and the debug server.
/// Updated every frame by the game loop. Read by the debug server on request.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DebugState {
    pub frame_count: u64,
    pub frame_time_ms: f64,
    pub fps: f64,
    pub entity_count: u32,
    pub window_width: u32,
    pub window_height: u32,
    pub uptime_seconds: f64,
    pub quit_requested: bool,
    /// Human-readable planetary coordinate string (e.g., "45.3°N, 122.1°W, 150m alt").
    pub planetary_position: String,
    /// Set to `true` by the debug server to request a screenshot capture.
    #[serde(skip)]
    pub screenshot_requested: bool,
    /// PNG-encoded screenshot data populated by the render loop.
    #[serde(skip)]
    pub screenshot_data: Option<Vec<u8>>,
}

/// Creates a new debug server in debug builds, returns None in release builds.
pub fn create_debug_server(port: u16) -> Option<DebugServer> {
    #[cfg(debug_assertions)]
    {
        Some(DebugServer::new(port))
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

/// Gets the debug port from environment variable or returns default.
pub fn get_debug_port() -> u16 {
    std::env::var("NEBULA_DEBUG_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(9999)
}
