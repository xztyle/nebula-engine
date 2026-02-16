//! Unit tests for the debug API.

use crate::{DebugServer, DebugState};
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[test]
fn test_debug_state_default() {
    let state = DebugState::default();
    assert_eq!(state.frame_count, 0);
    assert_eq!(state.fps, 0.0);
    assert_eq!(state.entity_count, 0);
    assert!(!state.quit_requested);
}

#[test]
fn test_debug_server_creation() {
    let server = DebugServer::new(0);
    assert_eq!(server.actual_port(), 0);
}

// NOTE: Additional server integration tests are intentionally omitted because
// tiny_http doesn't support clean shutdown, causing tests to hang indefinitely.
// The debug API functionality is validated through:
// 1. Manual testing with curl commands
// 2. Integration with the demo application
// 3. The demo window title shows "[Debug API :9999]" when active

#[test]
fn test_debug_server_starts_and_responds() {
    let state = Arc::new(Mutex::new(DebugState::default()));
    let mut server = DebugServer::new(0); // port 0 = OS assigns
    server.start(state).unwrap();

    // Give server a moment to start
    thread::sleep(Duration::from_millis(100));

    // GET /health should return 200
    let port = server.actual_port();
    let resp = ureq::get(&format!("http://localhost:{}/health", port))
        .call()
        .unwrap();
    assert_eq!(resp.status(), 200);

    let body_text = resp.into_string().unwrap();
    let body: serde_json::Value = serde_json::from_str(&body_text).unwrap();
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
        quit_requested: false,
        screenshot_requested: false,
        screenshot_data: None,
        planetary_position: String::new(),
    }));
    let mut server = DebugServer::new(0);
    server.start(state).unwrap();

    // Give server a moment to start
    thread::sleep(Duration::from_millis(100));

    let port = server.actual_port();
    let resp = ureq::get(&format!("http://localhost:{}/metrics", port))
        .call()
        .unwrap();
    assert_eq!(resp.status(), 200);

    let body_text = resp.into_string().unwrap();
    let body: serde_json::Value = serde_json::from_str(&body_text).unwrap();
    assert_eq!(body["frame_count"], 100);
    assert!((body["fps"].as_f64().unwrap() - 60.2).abs() < 0.01);
    assert_eq!(body["entity_count"], 5);
    assert_eq!(body["window_width"], 1920);
    assert_eq!(body["window_height"], 1080);
    server.stop();
}

#[test]
fn test_command_quit() {
    let state = Arc::new(Mutex::new(DebugState::default()));
    let mut server = DebugServer::new(0);
    server.start(state.clone()).unwrap();

    // Give server a moment to start
    thread::sleep(Duration::from_millis(100));

    let port = server.actual_port();
    let resp = ureq::post(&format!("http://localhost:{}/command", port))
        .set("Content-Type", "application/json")
        .send_string(r#"{"command": "quit"}"#)
        .unwrap();
    assert_eq!(resp.status(), 200);

    let body_text = resp.into_string().unwrap();
    let body: serde_json::Value = serde_json::from_str(&body_text).unwrap();
    assert_eq!(body["executed"], true);
    assert_eq!(body["command"], "quit");

    // Verify quit flag was set
    let debug_state = state.lock().unwrap();
    assert!(debug_state.quit_requested);
    server.stop();
}

#[test]
fn test_screenshot_returns_png() {
    let state = Arc::new(Mutex::new(DebugState::default()));
    let mut server = DebugServer::new(0);
    server.start(state.clone()).unwrap();

    // Give server a moment to start
    thread::sleep(Duration::from_millis(100));

    // Simulate a render loop that provides screenshot data when requested.
    let sim_state = state.clone();
    let _sim_thread = thread::spawn(move || {
        // Create a minimal valid PNG to serve as fake screenshot data
        let fake_png: Vec<u8> = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
            0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
            0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1x1
            0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, // RGBA 8bit
            0x00, 0x00, 0x00, 0x0B, 0x49, 0x44, 0x41, 0x54, // IDAT
            0x08, 0xD7, 0x63, 0x60, 0x00, 0x02, 0x00, 0x00, 0x05, 0x00, 0x01, // data
            0x0D, 0x0A, 0x2D, 0xB4, // CRC
            0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, // IEND
            0xAE, 0x42, 0x60, 0x82, // CRC
        ];
        loop {
            thread::sleep(Duration::from_millis(5));
            if let Ok(mut s) = sim_state.lock()
                && s.screenshot_requested
            {
                s.screenshot_data = Some(fake_png.clone());
            }
        }
    });

    let port = server.actual_port();
    let resp = ureq::get(&format!("http://localhost:{}/screenshot", port))
        .call()
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.header("Content-Type").unwrap(), "image/png");

    let mut bytes = Vec::new();
    resp.into_reader().read_to_end(&mut bytes).unwrap();
    assert!(!bytes.is_empty());

    // PNG magic bytes
    assert_eq!(&bytes[0..4], &[0x89, 0x50, 0x4E, 0x47]);
    server.stop();
}

#[test]
fn test_unknown_endpoint_returns_404() {
    let state = Arc::new(Mutex::new(DebugState::default()));
    let mut server = DebugServer::new(0);
    server.start(state).unwrap();

    // Give server a moment to start
    thread::sleep(Duration::from_millis(100));

    let port = server.actual_port();
    let resp = ureq::get(&format!("http://localhost:{}/nonexistent", port)).call();

    // ureq returns an error for 4xx/5xx status codes
    assert!(resp.is_err());
    if let Err(ureq::Error::Status(code, _)) = resp {
        assert_eq!(code, 404);
    } else {
        panic!("Expected 404 status error");
    }

    server.stop();
}
