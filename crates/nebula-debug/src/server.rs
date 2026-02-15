//! HTTP debug server implementation.

use crate::DebugState;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use tiny_http::{Header, Method, Request, Response, Server};

#[derive(Debug, thiserror::Error)]
pub enum DebugServerError {
    #[error("Failed to bind to port {port}: {error}")]
    BindError { port: u16, error: String },
    #[error("Server thread panicked")]
    ThreadPanic,
}

/// HTTP server for the debug API.
/// Runs on a background thread to avoid blocking the game loop.
pub struct DebugServer {
    port: u16,
    actual_port: Option<u16>,
    handle: Option<JoinHandle<()>>,
    shutdown_flag: Arc<Mutex<bool>>,
}

#[derive(Serialize, Deserialize)]
struct InputEvent {
    #[serde(rename = "type")]
    event_type: String,
    key: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Command {
    command: String,
}

#[derive(Serialize)]
struct CommandResponse {
    executed: bool,
    command: String,
}

#[derive(Serialize)]
struct InputResponse {
    accepted: bool,
}

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    uptime_seconds: f64,
}

#[derive(Serialize)]
struct StateResponse {
    entities: u32,
    components: Vec<String>,
}

impl DebugServer {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            actual_port: None,
            handle: None,
            shutdown_flag: Arc::new(Mutex::new(false)),
        }
    }

    pub fn start(&mut self, state: Arc<Mutex<DebugState>>) -> Result<(), DebugServerError> {
        let server = Server::http(format!("127.0.0.1:{}", self.port)).map_err(|e| {
            DebugServerError::BindError {
                port: self.port,
                error: e.to_string(),
            }
        })?;

        let actual_port = server
            .server_addr()
            .to_ip()
            .map(|addr| addr.port())
            .unwrap_or(self.port);
        self.actual_port = Some(actual_port);

        let shutdown_flag = self.shutdown_flag.clone();
        let handle = thread::spawn(move || {
            Self::run_server(server, state, shutdown_flag);
        });

        self.handle = Some(handle);
        Ok(())
    }

    pub fn stop(&mut self) {
        // tiny_http doesn't support graceful shutdown, so we just detach the thread.
        // The thread will terminate when the server is dropped or the process ends.
        if let Some(handle) = self.handle.take() {
            // Don't wait for the thread to join as it may be blocked in incoming_requests()
            std::mem::forget(handle);
        }
    }

    pub fn actual_port(&self) -> u16 {
        self.actual_port.unwrap_or(self.port)
    }

    fn run_server(server: Server, state: Arc<Mutex<DebugState>>, _shutdown_flag: Arc<Mutex<bool>>) {
        // Note: tiny_http doesn't support clean shutdown, but the server
        // will be dropped when the DebugServer instance is dropped, which
        // should terminate the thread naturally.
        for request in server.incoming_requests() {
            if let Err(e) = Self::handle_request(request, &state) {
                eprintln!("Debug server error: {}", e);
            }
        }
    }

    fn handle_request(
        mut request: Request,
        state: &Arc<Mutex<DebugState>>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let response = match (request.method(), request.url()) {
            (&Method::Get, "/health") => {
                let debug_state = state.lock().unwrap();
                let response = HealthResponse {
                    status: "ok".to_string(),
                    uptime_seconds: debug_state.uptime_seconds,
                };
                let json = serde_json::to_string(&response)?;
                Response::from_string(json).with_header(
                    Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
                )
            }
            (&Method::Get, "/metrics") => {
                let debug_state = state.lock().unwrap();
                let json = serde_json::to_string(&*debug_state)?;
                Response::from_string(json).with_header(
                    Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
                )
            }
            (&Method::Get, "/screenshot") => {
                // Return a 1x1 placeholder PNG
                let png_bytes = Self::create_placeholder_png();
                Response::from_data(png_bytes).with_header(
                    Header::from_bytes(&b"Content-Type"[..], &b"image/png"[..]).unwrap(),
                )
            }
            (&Method::Post, "/input") => {
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body)?;
                let _input_event: InputEvent = serde_json::from_str(&body)?;

                // Accept but don't process input (input system doesn't exist yet)
                let response = InputResponse { accepted: true };
                let json = serde_json::to_string(&response)?;
                Response::from_string(json).with_header(
                    Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
                )
            }
            (&Method::Post, "/command") => {
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body)?;
                let command: Command = serde_json::from_str(&body)?;

                let executed = match command.command.as_str() {
                    "quit" => {
                        // Set quit flag in state
                        if let Ok(mut debug_state) = state.lock() {
                            debug_state.quit_requested = true;
                        }
                        true
                    }
                    _ => false,
                };

                let response = CommandResponse {
                    executed,
                    command: command.command,
                };
                let json = serde_json::to_string(&response)?;
                Response::from_string(json).with_header(
                    Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
                )
            }
            (&Method::Get, "/state") => {
                let response = StateResponse {
                    entities: 0,
                    components: vec![],
                };
                let json = serde_json::to_string(&response)?;
                Response::from_string(json).with_header(
                    Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
                )
            }
            _ => Response::from_string("Not Found").with_status_code(404),
        };

        request.respond(response)?;
        Ok(())
    }

    /// Creates a minimal 1x1 PNG placeholder.
    fn create_placeholder_png() -> Vec<u8> {
        // PNG signature + minimal IHDR + IDAT + IEND chunks for a 1x1 transparent pixel
        vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
            0x00, 0x00, 0x00, 0x0D, // IHDR length
            0x49, 0x48, 0x44, 0x52, // IHDR
            0x00, 0x00, 0x00, 0x01, // width: 1
            0x00, 0x00, 0x00, 0x01, // height: 1
            0x08, 0x06, 0x00, 0x00,
            0x00, // bit depth 8, color type 6 (RGBA), compression 0, filter 0, interlace 0
            0x1F, 0x15, 0xC4, 0x89, // IHDR CRC
            0x00, 0x00, 0x00, 0x0B, // IDAT length
            0x49, 0x44, 0x41, 0x54, // IDAT
            0x08, 0xD7, 0x63, 0x60, 0x00, 0x02, 0x00, 0x00, 0x05, 0x00,
            0x01, // deflated data for transparent pixel
            0x0D, 0x0A, 0x2D, 0xB4, // IDAT CRC
            0x00, 0x00, 0x00, 0x00, // IEND length
            0x49, 0x45, 0x4E, 0x44, // IEND
            0xAE, 0x42, 0x60, 0x82, // IEND CRC
        ]
    }
}

impl Drop for DebugServer {
    fn drop(&mut self) {
        self.stop();
    }
}
