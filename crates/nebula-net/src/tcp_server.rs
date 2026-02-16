//! TCP server for accepting and managing client connections.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::{RwLock, watch};

/// Unique identifier for a TCP connection within a server session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub u64);

/// Atomic generator for monotonically increasing [`ConnectionId`]s.
pub struct IdGenerator {
    next: AtomicU64,
}

impl IdGenerator {
    /// Create a new generator starting at 1.
    pub fn new() -> Self {
        Self {
            next: AtomicU64::new(1),
        }
    }

    /// Return the next unique [`ConnectionId`].
    pub fn next_id(&self) -> ConnectionId {
        ConnectionId(self.next.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for IdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Error returned when the connection map is at capacity.
#[derive(Debug)]
pub struct ConnectionLimitReached;

/// Thread-safe map of active connections keyed by [`ConnectionId`].
pub struct ConnectionMap {
    inner: RwLock<HashMap<ConnectionId, OwnedWriteHalf>>,
    max_connections: usize,
}

impl ConnectionMap {
    /// Create a new map with the given capacity limit.
    pub fn new(max_connections: usize) -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            max_connections,
        }
    }

    /// Insert a connection. Returns `Err` if the map is at capacity.
    pub async fn insert(
        &self,
        id: ConnectionId,
        writer: OwnedWriteHalf,
    ) -> Result<(), ConnectionLimitReached> {
        let mut map = self.inner.write().await;
        if map.len() >= self.max_connections {
            return Err(ConnectionLimitReached);
        }
        map.insert(id, writer);
        Ok(())
    }

    /// Remove a connection by ID.
    pub async fn remove(&self, id: &ConnectionId) -> Option<OwnedWriteHalf> {
        self.inner.write().await.remove(id)
    }

    /// Return the number of active connections.
    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }

    /// Return whether the map is empty.
    pub async fn is_empty(&self) -> bool {
        self.inner.read().await.is_empty()
    }
}

/// Configuration for [`GameServer`].
pub struct ServerConfig {
    /// Address to bind to. Default: `0.0.0.0:7777`.
    pub bind_addr: SocketAddr,
    /// Maximum concurrent connections. Default: 256.
    pub max_connections: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:7777".parse().unwrap(),
            max_connections: 256,
        }
    }
}

/// TCP game server that accepts connections and manages their lifecycle.
pub struct GameServer {
    config: ServerConfig,
    /// Active connection map (public for test inspection).
    pub connections: Arc<ConnectionMap>,
    id_gen: Arc<IdGenerator>,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}

impl GameServer {
    /// Create a new server with the given configuration.
    pub fn new(config: ServerConfig) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            connections: Arc::new(ConnectionMap::new(config.max_connections)),
            id_gen: Arc::new(IdGenerator::new()),
            config,
            shutdown_tx,
            shutdown_rx,
        }
    }

    /// Bind to the configured address and run the accept loop.
    pub async fn run(&self) -> std::io::Result<()> {
        let listener = TcpListener::bind(self.config.bind_addr).await?;
        tracing::info!("Server listening on {}", self.config.bind_addr);
        self.run_with_listener(listener).await
    }

    /// Run the accept loop with a pre-bound listener (useful for tests).
    pub async fn run_with_listener(&self, listener: TcpListener) -> std::io::Result<()> {
        let mut shutdown_rx = self.shutdown_rx.clone();

        loop {
            tokio::select! {
                result = listener.accept() => {
                    let (stream, peer_addr) = result?;
                    stream.set_nodelay(true)?;

                    let id = self.id_gen.next_id();
                    let (reader, writer) = stream.into_split();

                    if self.connections.insert(id, writer).await.is_err() {
                        tracing::warn!("Connection limit reached, rejecting {peer_addr}");
                        continue;
                    }

                    tracing::info!("Accepted connection {id:?} from {peer_addr}");

                    let connections = Arc::clone(&self.connections);
                    let mut task_shutdown = self.shutdown_rx.clone();

                    tokio::spawn(async move {
                        Self::handle_connection(id, reader, &mut task_shutdown).await;
                        connections.remove(&id).await;
                        tracing::info!("Connection {id:?} closed");
                    });
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        tracing::info!("Server shutting down");
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    /// Signal the server to shut down gracefully.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Per-connection reader loop.
    async fn handle_connection(
        id: ConnectionId,
        mut reader: tokio::net::tcp::OwnedReadHalf,
        shutdown_rx: &mut watch::Receiver<bool>,
    ) {
        let mut buf = [0u8; 4096];
        loop {
            tokio::select! {
                result = reader.read(&mut buf) => {
                    match result {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            // Future: pass bytes to framing layer
                            let _ = &buf[..n];
                            tracing::trace!("Connection {id:?} received {n} bytes");
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpStream;

    /// Helper: start a server on an ephemeral port and return the bound address.
    async fn start_test_server(max_connections: usize) -> (SocketAddr, Arc<GameServer>) {
        let config = ServerConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            max_connections,
        };
        let server = Arc::new(GameServer::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let srv = Arc::clone(&server);
        tokio::spawn(async move {
            srv.run_with_listener(listener).await.unwrap();
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        (addr, server)
    }

    #[tokio::test]
    async fn test_server_binds_to_port() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        assert_ne!(addr.port(), 0);
    }

    #[tokio::test]
    async fn test_server_accepts_connection() {
        let (addr, _server) = start_test_server(16).await;
        let stream = TcpStream::connect(addr).await;
        assert!(stream.is_ok(), "Client should connect to the server");
    }

    #[tokio::test]
    async fn test_multiple_clients_connect() {
        let (addr, server) = start_test_server(16).await;
        let mut streams = Vec::new();
        for _ in 0..5 {
            let stream = TcpStream::connect(addr).await.unwrap();
            streams.push(stream);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(server.connections.len().await, 5);
    }

    #[tokio::test]
    async fn test_max_connections_enforced() {
        let max = 2;
        let (addr, server) = start_test_server(max).await;

        let _c1 = TcpStream::connect(addr).await.unwrap();
        let _c2 = TcpStream::connect(addr).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(server.connections.len().await, 2);

        let _c3 = TcpStream::connect(addr).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(server.connections.len().await <= max);
    }

    #[tokio::test]
    async fn test_graceful_shutdown_closes_connections() {
        let (addr, server) = start_test_server(16).await;
        let mut stream = TcpStream::connect(addr).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        server.shutdown();
        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut buf = [0u8; 64];
        let n = stream.read(&mut buf).await.unwrap();
        assert_eq!(n, 0, "Client should receive EOF after server shutdown");
    }

    #[tokio::test]
    async fn test_connection_id_uniqueness() {
        let id_gen = IdGenerator::new();
        let id1 = id_gen.next_id();
        let id2 = id_gen.next_id();
        let id3 = id_gen.next_id();
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_eq!(id1.0 + 1, id2.0);
        assert_eq!(id2.0 + 1, id3.0);
    }
}
