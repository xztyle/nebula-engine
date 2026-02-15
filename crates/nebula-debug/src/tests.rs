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
    server.start(state).unwrap();

    // Give server a moment to start
    thread::sleep(Duration::from_millis(100));

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
