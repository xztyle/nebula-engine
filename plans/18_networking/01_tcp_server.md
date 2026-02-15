# TCP Server

## Problem

The Nebula Engine needs a dedicated game server that accepts incoming TCP connections from clients. Since the engine uses 128-bit coordinates for its cubesphere-voxel planets, the server is the authoritative source of truth for world state and must handle many simultaneous players. Without a well-structured TCP server, there is no foundation for multiplayer. The server must bind to a configurable address, manage the lifecycle of each connection, enforce a cap on concurrent connections to protect resource usage, and shut down cleanly without dropping in-flight data. Tokio provides the async runtime, but the connection management layer — tracking who is connected, assigning stable identifiers, and spawning per-connection tasks — must be built explicitly.

## Solution

### Connection identity

Each accepted TCP connection is assigned a monotonically increasing `ConnectionId` (a `u64`). The counter lives in the server and is incremented atomically with `AtomicU64`. The ID is stable for the lifetime of the connection and is never reused within a single server session.

```rust
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub u64);

pub struct IdGenerator {
    next: AtomicU64,
}

impl IdGenerator {
    pub fn new() -> Self {
        Self { next: AtomicU64::new(1) }
    }

    pub fn next_id(&self) -> ConnectionId {
        ConnectionId(self.next.fetch_add(1, Ordering::Relaxed))
    }
}
```

### Connection map

Active connections are stored in a concurrent map protected by a `tokio::sync::RwLock`. The map holds split halves of each `TcpStream` — the `OwnedWriteHalf` is kept in the map so the server can send messages to any client by ID, while the `OwnedReadHalf` is moved into the per-connection reader task.

```rust
use std::collections::HashMap;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::RwLock;

pub struct ConnectionMap {
    inner: RwLock<HashMap<ConnectionId, OwnedWriteHalf>>,
    max_connections: usize,
}

impl ConnectionMap {
    pub fn new(max_connections: usize) -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            max_connections,
        }
    }

    /// Insert a new connection. Returns `Err` if the map is at capacity.
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

    /// Remove a connection by ID, returning the writer half if it existed.
    pub async fn remove(&self, id: &ConnectionId) -> Option<OwnedWriteHalf> {
        self.inner.write().await.remove(id)
    }

    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }
}

#[derive(Debug)]
pub struct ConnectionLimitReached;
```

### Server configuration

```rust
use std::net::SocketAddr;

pub struct ServerConfig {
    /// Address to bind to. Default: 0.0.0.0:7777
    pub bind_addr: SocketAddr,
    /// Maximum number of concurrent connections. Default: 256
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
```

### Server loop

`GameServer::start` binds a `TcpListener`, then enters an accept loop. A `tokio::sync::watch` channel carries the shutdown signal — when the server handle is dropped or `shutdown()` is called, the watch value flips to `true` and the accept loop exits. Each accepted connection is split into reader/writer halves. The writer goes into the `ConnectionMap`, the reader is moved into a spawned task.

```rust
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::watch;

pub struct GameServer {
    config: ServerConfig,
    connections: Arc<ConnectionMap>,
    id_gen: Arc<IdGenerator>,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}

impl GameServer {
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

    pub async fn run(&self) -> std::io::Result<()> {
        let listener = TcpListener::bind(self.config.bind_addr).await?;
        let mut shutdown_rx = self.shutdown_rx.clone();

        loop {
            tokio::select! {
                result = listener.accept() => {
                    let (stream, peer_addr) = result?;
                    stream.set_nodelay(true)?;

                    let id = self.id_gen.next_id();
                    let (reader, writer) = stream.into_split();

                    if self.connections.insert(id, writer).await.is_err() {
                        // At capacity — drop the connection immediately.
                        tracing::warn!("Connection limit reached, rejecting {peer_addr}");
                        continue;
                    }

                    tracing::info!("Accepted connection {id:?} from {peer_addr}");

                    let connections = Arc::clone(&self.connections);
                    let mut task_shutdown = self.shutdown_rx.clone();

                    tokio::spawn(async move {
                        Self::handle_connection(id, reader, &connections, &mut task_shutdown).await;
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

    async fn handle_connection(
        id: ConnectionId,
        mut reader: tokio::net::tcp::OwnedReadHalf,
        connections: &ConnectionMap,
        shutdown_rx: &mut watch::Receiver<bool>,
    ) {
        use tokio::io::AsyncReadExt;
        let mut buf = [0u8; 4096];
        loop {
            tokio::select! {
                result = reader.read(&mut buf) => {
                    match result {
                        Ok(0) | Err(_) => break, // Connection closed or error
                        Ok(n) => {
                            // Pass bytes to framing layer (story 03)
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
}
```

### Graceful shutdown

When `shutdown()` is called, the accept loop exits. Each per-connection task also monitors the shutdown watch channel and terminates its read loop. The `ConnectionMap` entries are removed as each task exits, which drops the `OwnedWriteHalf` and sends a TCP FIN to the client. This ensures every connection is closed cleanly.

## Outcome

A `tcp_server.rs` module in `crates/nebula_net/src/` exporting `GameServer`, `ServerConfig`, `ConnectionId`, `ConnectionMap`, and `IdGenerator`. The server binds to a configurable address, accepts TCP connections into a concurrent map, enforces a maximum connection limit, and shuts down gracefully via a watch channel. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Running `nebula-demo --server` starts a TCP listener on port 7777. The console shows `Server listening on 0.0.0.0:7777`. The demo gains a server mode.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | `1.49` (features: `net`, `rt-multi-thread`, `io-util`, `macros`) | Async TCP listener, stream splitting, task spawning, select macro |
| `tracing` | `0.1` | Structured logging for connection events |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use std::time::Duration;

    /// Helper: start a server on an ephemeral port and return the bound address.
    async fn start_test_server(max_connections: usize) -> (SocketAddr, Arc<GameServer>) {
        let config = ServerConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            max_connections,
        };
        let server = Arc::new(GameServer::new(config));
        // We need the actual bound address, so bind inside run or use a
        // separate method. For tests, bind manually:
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let srv = Arc::clone(&server);
        tokio::spawn(async move {
            srv.run_with_listener(listener).await.unwrap();
        });
        // Small yield to ensure the accept loop is running
        tokio::time::sleep(Duration::from_millis(10)).await;
        (addr, server)
    }

    #[tokio::test]
    async fn test_server_binds_to_port() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        assert_ne!(addr.port(), 0);
        // Port is valid and non-zero, meaning the bind succeeded.
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

        // Fill to capacity
        let _c1 = TcpStream::connect(addr).await.unwrap();
        let _c2 = TcpStream::connect(addr).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(server.connections.len().await, 2);

        // Third connection should be accepted at TCP level but immediately
        // dropped by the server because the ConnectionMap is full.
        let c3 = TcpStream::connect(addr).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        // The server map should still be at capacity (the third was rejected).
        assert!(server.connections.len().await <= max);
    }

    #[tokio::test]
    async fn test_graceful_shutdown_closes_connections() {
        let (addr, server) = start_test_server(16).await;
        let mut stream = TcpStream::connect(addr).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Trigger shutdown
        server.shutdown();
        tokio::time::sleep(Duration::from_millis(100)).await;

        // After shutdown the server should have dropped all writer halves,
        // so a read on the client should return 0 bytes (EOF).
        let mut buf = [0u8; 64];
        let n = stream.read(&mut buf).await.unwrap();
        assert_eq!(n, 0, "Client should receive EOF after server shutdown");
    }

    #[tokio::test]
    async fn test_connection_id_uniqueness() {
        let gen = IdGenerator::new();
        let id1 = gen.next_id();
        let id2 = gen.next_id();
        let id3 = gen.next_id();
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_eq!(id1.0 + 1, id2.0);
        assert_eq!(id2.0 + 1, id3.0);
    }
}
```
