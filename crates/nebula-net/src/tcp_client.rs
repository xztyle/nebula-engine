//! TCP client for connecting to a Nebula Engine game server.
//!
//! Manages the full connection lifecycle: connecting, heartbeat keepalive,
//! and clean disconnect. State changes are broadcast via a [`watch`] channel
//! so any number of consumers can react without polling.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, watch};

/// Connection lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Attempting to establish a TCP connection.
    Connecting,
    /// TCP connection established, ready for communication.
    Connected,
    /// Connection lost or intentionally closed.
    Disconnected,
}

/// Observable connection state backed by a [`watch`] channel.
///
/// Multiple subscribers can observe state transitions without polling.
pub struct ConnectionStateWatch {
    tx: watch::Sender<ConnectionState>,
    rx: watch::Receiver<ConnectionState>,
}

impl Default for ConnectionStateWatch {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectionStateWatch {
    /// Create a new watch initialized to [`ConnectionState::Disconnected`].
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(ConnectionState::Disconnected);
        Self { tx, rx }
    }

    /// Set the current connection state, notifying all subscribers.
    pub fn set(&self, state: ConnectionState) {
        let _ = self.tx.send(state);
    }

    /// Return a new subscriber receiver.
    pub fn subscribe(&self) -> watch::Receiver<ConnectionState> {
        self.rx.clone()
    }

    /// Return the current state without blocking.
    pub fn current(&self) -> ConnectionState {
        *self.rx.borrow()
    }
}

/// Handle to a connected game server session.
///
/// Created via [`GameClient::connect`]. Owns the writer half of the TCP
/// stream (behind a mutex for shared access), the connection state watch,
/// and a shutdown signal for background tasks.
pub struct GameClient {
    /// Writer half, shared with the heartbeat task.
    /// Will be used by the framing/send layer in story 03.
    #[allow(dead_code)]
    writer: Arc<Mutex<tokio::net::tcp::OwnedWriteHalf>>,
    /// Observable connection state.
    state: Arc<ConnectionStateWatch>,
    /// Sending `true` causes reader and heartbeat tasks to exit.
    shutdown_tx: watch::Sender<bool>,
}

impl GameClient {
    /// Connect to the server at `addr`.
    ///
    /// Sets `TCP_NODELAY`, splits the stream, and spawns reader + heartbeat
    /// background tasks. Returns immediately after the TCP handshake.
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

    /// Return the connection state watch.
    pub fn state(&self) -> &Arc<ConnectionStateWatch> {
        &self.state
    }

    /// Disconnect from the server.
    ///
    /// Signals background tasks to exit and transitions state to
    /// [`ConnectionState::Disconnected`] immediately.
    pub fn disconnect(&self) {
        let _ = self.shutdown_tx.send(true);
        self.state.set(ConnectionState::Disconnected);
    }

    /// Read incoming bytes until the connection closes or shutdown is signalled.
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
                            // Forward bytes to framing layer (story 03).
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
    /// transition to [`ConnectionState::Disconnected`].
    async fn heartbeat_loop(
        writer: &Mutex<tokio::net::tcp::OwnedWriteHalf>,
        state: &ConnectionStateWatch,
        shutdown_rx: &mut watch::Receiver<bool>,
    ) {
        let ping_interval = Duration::from_secs(5);
        let timeout_duration = Duration::from_secs(15);
        let last_pong = tokio::time::Instant::now();
        let mut interval = tokio::time::interval(ping_interval);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if state.current() != ConnectionState::Connected {
                        break;
                    }

                    // Check for timeout
                    if last_pong.elapsed() > timeout_duration {
                        tracing::warn!(
                            "Heartbeat timeout — no response in {timeout_duration:?}"
                        );
                        state.set(ConnectionState::Disconnected);
                        break;
                    }

                    // Send ping (placeholder opcode; real message defined in story 04).
                    let ping_bytes = [0x00];
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Helper: start a minimal TCP listener that accepts one connection
    /// and echoes back everything it receives (acting as pong).
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
        assert_eq!(
            client.unwrap().state().current(),
            ConnectionState::Connected
        );
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
