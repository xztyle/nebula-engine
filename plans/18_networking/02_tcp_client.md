# TCP Client

## Problem

Players need a way to connect to a Nebula Engine game server and maintain a persistent TCP connection for the duration of their session. The client must manage the full connection lifecycle — initiating the connection, maintaining it via heartbeats, and handling unexpected disconnects. Without a heartbeat mechanism, neither side can distinguish between a slow connection and a dead one, leading to ghost connections that waste server resources. The client must expose connection state changes as events so that the game UI and ECS systems can react (show "Connecting..." overlay, transition to gameplay on connect, show "Disconnected" on drop). The client runs on Linux, Windows, and macOS, so all socket operations go through tokio's cross-platform abstractions.

## Solution

### Connection state

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Attempting to establish a TCP connection.
    Connecting,
    /// TCP connection established, ready for communication.
    Connected,
    /// Connection lost or intentionally closed.
    Disconnected,
}
```

### Connection state events

State transitions are broadcast via a `tokio::sync::watch` channel so that any number of consumers (UI system, ECS bridge, logging) can observe the current state without polling.

```rust
use tokio::sync::watch;

pub struct ConnectionStateWatch {
    tx: watch::Sender<ConnectionState>,
    rx: watch::Receiver<ConnectionState>,
}

impl ConnectionStateWatch {
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(ConnectionState::Disconnected);
        Self { tx, rx }
    }

    pub fn set(&self, state: ConnectionState) {
        let _ = self.tx.send(state);
    }

    pub fn subscribe(&self) -> watch::Receiver<ConnectionState> {
        self.rx.clone()
    }

    pub fn current(&self) -> ConnectionState {
        *self.rx.borrow()
    }
}
```

### Client handle

`GameClient::connect` performs the TCP connection, sets `TCP_NODELAY`, splits the stream, and spawns reader and heartbeat tasks. The returned `GameClient` holds the writer half (behind a mutex for shared access), the state watch, and a shutdown channel.

```rust
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{watch, Mutex};

pub struct GameClient {
    writer: Arc<Mutex<tokio::net::tcp::OwnedWriteHalf>>,
    state: Arc<ConnectionStateWatch>,
    shutdown_tx: watch::Sender<bool>,
}

impl GameClient {
    /// Connect to the server at the given address.
    pub async fn connect(addr: SocketAddr) -> std::io::Result<Self> {
        let state = Arc::new(ConnectionStateWatch::new());
        state.set(ConnectionState::Connecting);

        let stream = TcpStream::connect(addr).await?;
        stream.set_nodelay(true)?;

        state.set(ConnectionState::Connected);

        let (reader, writer) = stream.into_split();
        let writer = Arc::new(Mutex::new(writer));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Spawn reader task
        let reader_state = Arc::clone(&state);
        let mut reader_shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            Self::read_loop(reader, &reader_state, &mut reader_shutdown).await;
        });

        // Spawn heartbeat task
        let hb_writer = Arc::clone(&writer);
        let hb_state = Arc::clone(&state);
        let mut hb_shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            Self::heartbeat_loop(&hb_writer, &hb_state, &mut hb_shutdown).await;
        });

        Ok(Self {
            writer,
            state,
            shutdown_tx,
        })
    }

    pub fn state(&self) -> &Arc<ConnectionStateWatch> {
        &self.state
    }

    /// Disconnect from the server.
    pub fn disconnect(&self) {
        let _ = self.shutdown_tx.send(true);
        self.state.set(ConnectionState::Disconnected);
    }

    async fn read_loop(
        mut reader: tokio::net::tcp::OwnedReadHalf,
        state: &ConnectionStateWatch,
        shutdown_rx: &mut watch::Receiver<bool>,
    ) {
        let mut buf = [0u8; 4096];
        loop {
            tokio::select! {
                result = reader.read(&mut buf) => {
                    match result {
                        Ok(0) | Err(_) => {
                            state.set(ConnectionState::Disconnected);
                            break;
                        }
                        Ok(n) => {
                            // Forward bytes to framing layer (story 03)
                            let _ = &buf[..n];
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
            }
        }
    }

    /// Send a ping every 5 seconds. If no pong is received within 15 seconds,
    /// consider the connection dead and transition to Disconnected.
    async fn heartbeat_loop(
        writer: &Mutex<tokio::net::tcp::OwnedWriteHalf>,
        state: &ConnectionStateWatch,
        shutdown_rx: &mut watch::Receiver<bool>,
    ) {
        let ping_interval = Duration::from_secs(5);
        let timeout_duration = Duration::from_secs(15);
        let mut last_pong = tokio::time::Instant::now();
        let mut interval = tokio::time::interval(ping_interval);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if state.current() != ConnectionState::Connected {
                        break;
                    }

                    // Check for timeout
                    if last_pong.elapsed() > timeout_duration {
                        tracing::warn!("Heartbeat timeout — no response in {timeout_duration:?}");
                        state.set(ConnectionState::Disconnected);
                        break;
                    }

                    // Send ping (the actual ping message is serialized via story 04)
                    let ping_bytes = [0x00]; // Placeholder ping opcode
                    let mut w = writer.lock().await;
                    if w.write_all(&ping_bytes).await.is_err() {
                        state.set(ConnectionState::Disconnected);
                        break;
                    }
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
            }
        }
    }
}
```

### Heartbeat protocol

The heartbeat is a lightweight ping/pong exchange. The client sends a `Ping` message every 5 seconds. The server responds with a `Pong`. If 15 seconds elapse since the last received `Pong`, the client considers the connection dead. The server performs the same check in reverse — if no `Ping` arrives from a client within 15 seconds, the server drops the connection. The specific message types are defined in story 04; this story defines the timing and state transition logic.

### Clean disconnect

Calling `GameClient::disconnect()` sends the shutdown signal, which causes both the reader and heartbeat tasks to exit. The writer half is dropped when the `GameClient` is dropped, sending a TCP FIN to the server. The state transitions to `Disconnected` immediately so the UI can react.

## Outcome

A `tcp_client.rs` module in `crates/nebula_net/src/` exporting `GameClient`, `ConnectionState`, and `ConnectionStateWatch`. The client connects to a server via TCP, maintains the connection with a ping/pong heartbeat, emits state changes via a watch channel, and disconnects cleanly. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Running the demo without `--server` connects as a client to `localhost:7777`. The console shows `Connecting to 127.0.0.1:7777... Connected!`.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | `1.49` (features: `net`, `rt-multi-thread`, `io-util`, `macros`) | Async TCP stream, split, interval timer, select macro |
| `tracing` | `0.1` | Structured logging for connection events and heartbeat timeouts |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use std::time::Duration;

    /// Helper: start a minimal TCP listener that accepts one connection
    /// and optionally responds to pings with pongs.
    async fn echo_server() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            loop {
                match stream.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        // Echo back as pong
                        let _ = stream.write_all(&buf[..n]).await;
                    }
                }
            }
        });
        addr
    }

    #[tokio::test]
    async fn test_client_connects_to_server() {
        let addr = echo_server().await;
        let client = GameClient::connect(addr).await;
        assert!(client.is_ok(), "Client should connect successfully");
        assert_eq!(client.unwrap().state().current(), ConnectionState::Connected);
    }

    #[tokio::test]
    async fn test_connection_state_starts_disconnected() {
        let watch = ConnectionStateWatch::new();
        assert_eq!(watch.current(), ConnectionState::Disconnected);
    }

    #[tokio::test]
    async fn test_connection_state_transitions() {
        let watch = ConnectionStateWatch::new();
        assert_eq!(watch.current(), ConnectionState::Disconnected);

        watch.set(ConnectionState::Connecting);
        assert_eq!(watch.current(), ConnectionState::Connecting);

        watch.set(ConnectionState::Connected);
        assert_eq!(watch.current(), ConnectionState::Connected);

        watch.set(ConnectionState::Disconnected);
        assert_eq!(watch.current(), ConnectionState::Disconnected);
    }

    #[tokio::test]
    async fn test_heartbeat_keeps_connection_alive() {
        let addr = echo_server().await;
        let client = GameClient::connect(addr).await.unwrap();
        // Wait longer than one heartbeat interval but less than timeout
        tokio::time::sleep(Duration::from_secs(7)).await;
        assert_eq!(
            client.state().current(),
            ConnectionState::Connected,
            "Connection should still be alive after one heartbeat cycle"
        );
    }

    #[tokio::test]
    async fn test_timeout_after_no_response() {
        // Start a server that accepts but never responds
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            // Hold the connection open but never read or write
            tokio::time::sleep(Duration::from_secs(60)).await;
            drop(stream);
        });

        let client = GameClient::connect(addr).await.unwrap();
        // Wait for heartbeat timeout (15s + margin)
        tokio::time::sleep(Duration::from_secs(18)).await;
        assert_eq!(
            client.state().current(),
            ConnectionState::Disconnected,
            "Connection should be marked disconnected after heartbeat timeout"
        );
    }

    #[tokio::test]
    async fn test_disconnect_is_clean() {
        let addr = echo_server().await;
        let client = GameClient::connect(addr).await.unwrap();
        assert_eq!(client.state().current(), ConnectionState::Connected);

        client.disconnect();
        assert_eq!(
            client.state().current(),
            ConnectionState::Disconnected,
            "State should transition to Disconnected immediately"
        );
    }

    #[tokio::test]
    async fn test_state_watch_subscriber_receives_updates() {
        let addr = echo_server().await;
        let client = GameClient::connect(addr).await.unwrap();
        let mut rx = client.state().subscribe();

        // Current value should be Connected
        assert_eq!(*rx.borrow(), ConnectionState::Connected);

        client.disconnect();
        // Wait for the change notification
        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow(), ConnectionState::Disconnected);
    }
}
```
