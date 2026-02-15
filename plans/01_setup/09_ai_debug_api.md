# AI Debug API

## Problem

The Nebula Engine's core differentiator is AI-friendly development. Without a built-in debug API, AI agents cannot observe the running engine, take screenshots, read metrics, inject inputs, or query game state. Every game built on Nebula must automatically expose this API in debug/test builds without any game-specific code. This must exist from the earliest possible moment so that all subsequent stories can be validated programmatically by AI agents rather than requiring human visual inspection.

## Solution

Create a `nebula-debug` HTTP server that starts automatically in debug builds. The server runs on a background thread (using a lightweight HTTP library) and exposes endpoints for AI observation and control.

### Debug Server

```rust
/// Starts the debug API server on the specified port.
/// Only compiled in debug builds (#[cfg(debug_assertions)]).
/// Runs on a dedicated background thread to avoid blocking the game loop.
pub struct DebugServer {
    port: u16,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl DebugServer {
    pub fn new(port: u16) -> Self { ... }
    pub fn start(&mut self, state: Arc<Mutex<DebugState>>) -> Result<(), DebugServerError> { ... }
    pub fn stop(&mut self) { ... }
}
```

### Shared State

```rust
/// State shared between the game loop and the debug server.
/// Updated every frame by the game loop. Read by the debug server on request.
pub struct DebugState {
    pub frame_count: u64,
    pub frame_time_ms: f64,
    pub fps: f64,
    pub entity_count: u32,
    pub window_width: u32,
    pub window_height: u32,
    pub uptime_seconds: f64,
}
```

### Endpoints

All endpoints return JSON unless otherwise specified.

- **`GET /health`** — Returns `{"status": "ok", "uptime_seconds": 42.5}`. Used by AI agents to verify the engine is running and responsive.

- **`GET /metrics`** — Returns current frame metrics:
  ```json
  {
    "frame_count": 1000,
    "frame_time_ms": 16.2,
    "fps": 61.7,
    "entity_count": 0,
    "window_width": 1280,
    "window_height": 720,
    "uptime_seconds": 16.4
  }
  ```

- **`GET /screenshot`** — Returns a PNG screenshot of the current frame. Initially returns a 1x1 placeholder PNG (rendering doesn't exist yet). Once rendering is implemented in later stories, this will capture the actual framebuffer.

- **`POST /input`** — Inject input events. Body is JSON:
  ```json
  {
    "type": "key_press",
    "key": "W"
  }
  ```
  Initially accepts and acknowledges input but does not process it (input system doesn't exist yet). Returns `{"accepted": true}`.

- **`POST /command`** — Execute engine commands. Body is JSON:
  ```json
  {
    "command": "quit"
  }
  ```
  Initially supports only `quit` (sets a shutdown flag). Returns `{"executed": true, "command": "quit"}`.

- **`GET /state`** — Query ECS state. Initially returns `{"entities": 0, "components": []}` since ECS isn't set up yet. Will be expanded as ECS stories are completed.

### HTTP Implementation

Use `tiny_http` as the HTTP server — it's minimal, has no async runtime requirement, and runs cleanly on a background std thread. No tokio or async complexity needed.

### Integration with Game Loop

The game loop updates `DebugState` via the shared `Arc<Mutex<DebugState>>` once per frame. The debug server reads it on each request. The mutex is held briefly (just a struct copy), so contention is negligible.

### Conditional Compilation

The entire debug server is behind `#[cfg(debug_assertions)]`. Release builds have zero overhead — no HTTP server, no shared state, no background thread.

```rust
#[cfg(debug_assertions)]
pub mod debug_api;

// In the game loop setup:
#[cfg(debug_assertions)]
{
    let debug_server = DebugServer::new(9999);
    debug_server.start(debug_state.clone())?;
}
```

### Default Port

The default port is `9999`. It can be overridden via the `NEBULA_DEBUG_PORT` environment variable.

## Outcome

Running `cargo run -p nebula-demo` in debug mode starts the HTTP debug API on port 9999. An AI agent (or curl) can:

- `curl http://localhost:9999/health` — verify engine is alive
- `curl http://localhost:9999/metrics` — read frame time and FPS
- `curl http://localhost:9999/screenshot` — get a (placeholder) screenshot
- `curl -X POST http://localhost:9999/command -d '{"command":"quit"}'` — shut down the engine

This foundation is expanded by every subsequent story as new systems come online.

## Demo Integration

**Demo crate:** `nebula-demo`

The demo starts with the debug API active. Running `curl localhost:9999/health` while the demo is open returns `{"status":"ok","uptime_seconds":...}`. The demo's window title includes `[Debug API :9999]`.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tiny_http` | `0.12` | Lightweight synchronous HTTP server |
| `serde` | `1` | JSON serialization for request/response types |
| `serde_json` | `1` | JSON parsing and generation |

These are added to the `nebula-debug` crate only. `serde` and `serde_json` are added as workspace dependencies since they'll be used widely.

## Unit Tests

```rust
#[test]
fn test_debug_state_default() {
    let state = DebugState::default();
    assert_eq!(state.frame_count, 0);
    assert_eq!(state.fps, 0.0);
    assert_eq!(state.entity_count, 0);
}

#[test]
fn test_debug_server_starts_and_responds() {
    let state = Arc::new(Mutex::new(DebugState::default()));
    let mut server = DebugServer::new(0); // port 0 = OS assigns
    server.start(state).unwrap();
    // GET /health should return 200
    let port = server.actual_port();
    let resp = reqwest::blocking::get(format!("http://localhost:{}/health", port)).unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().unwrap();
    assert_eq!(body["status"], "ok");
    server.stop();
}

#[test]
fn test_metrics_endpoint_returns_valid_json() {
    let state = Arc::new(Mutex::new(DebugState {
        frame_count: 100,
        frame_time_ms: 16.6,
        fps: 60.2,
        entity_count: 5,
        window_width: 1920,
        window_height: 1080,
        uptime_seconds: 1.66,
    }));
    let mut server = DebugServer::new(0);
    server.start(state).unwrap();
    let port = server.actual_port();
    let resp = reqwest::blocking::get(format!("http://localhost:{}/metrics", port)).unwrap();
    let body: serde_json::Value = resp.json().unwrap();
    assert_eq!(body["frame_count"], 100);
    assert!((body["fps"].as_f64().unwrap() - 60.2).abs() < 0.01);
    server.stop();
}

#[test]
fn test_command_quit() {
    let state = Arc::new(Mutex::new(DebugState::default()));
    let mut server = DebugServer::new(0);
    server.start(state.clone()).unwrap();
    let port = server.actual_port();
    let client = reqwest::blocking::Client::new();
    let resp = client.post(format!("http://localhost:{}/command", port))
        .json(&serde_json::json!({"command": "quit"}))
        .send().unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().unwrap();
    assert_eq!(body["executed"], true);
    server.stop();
}

#[test]
fn test_screenshot_returns_png() {
    let state = Arc::new(Mutex::new(DebugState::default()));
    let mut server = DebugServer::new(0);
    server.start(state).unwrap();
    let port = server.actual_port();
    let resp = reqwest::blocking::get(format!("http://localhost:{}/screenshot", port)).unwrap();
    assert_eq!(resp.headers().get("content-type").unwrap(), "image/png");
    let bytes = resp.bytes().unwrap();
    assert!(bytes.len() > 0);
    // PNG magic bytes
    assert_eq!(&bytes[0..4], &[0x89, 0x50, 0x4E, 0x47]);
    server.stop();
}

#[test]
fn test_unknown_endpoint_returns_404() {
    let state = Arc::new(Mutex::new(DebugState::default()));
    let mut server = DebugServer::new(0);
    server.start(state).unwrap();
    let port = server.actual_port();
    let resp = reqwest::blocking::get(format!("http://localhost:{}/nonexistent", port)).unwrap();
    assert_eq!(resp.status(), 404);
    server.stop();
}

## Performance Validation

The debug server must add less than 0.1ms of overhead per frame (just a mutex lock + struct copy). Measure with and without the debug server to verify.
```
