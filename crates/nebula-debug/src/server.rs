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
                // Request a screenshot from the render loop
                {
                    let mut s = state.lock().unwrap();
                    s.screenshot_data = None;
                    s.screenshot_requested = true;
                }

                // Poll for up to 500ms waiting for the render loop to provide data
                let mut png_data: Option<Vec<u8>> = None;
                for _ in 0..50 {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    let mut s = state.lock().unwrap();
                    if let Some(data) = s.screenshot_data.take() {
                        s.screenshot_requested = false;
                        png_data = Some(data);
                        break;
                    }
                }

                if let Some(data) = png_data {
                    Response::from_data(data).with_header(
                        Header::from_bytes(&b"Content-Type"[..], &b"image/png"[..]).unwrap(),
                    )
                } else {
                    // Timeout — clear request flag and return 503
                    if let Ok(mut s) = state.lock() {
                        s.screenshot_requested = false;
                    }
                    Response::from_string("Screenshot capture timed out").with_status_code(503)
                }
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
}

impl Drop for DebugServer {
    fn drop(&mut self) {
        self.stop();
    }
}
