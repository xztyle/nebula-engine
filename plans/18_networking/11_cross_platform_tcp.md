# Cross-Platform TCP

## Problem

The Nebula Engine targets Linux, Windows, and macOS. While tokio abstracts most platform differences in TCP networking, several socket-level behaviors vary across operating systems and must be explicitly configured for correct game networking behavior. **TCP_NODELAY** (Nagle's algorithm): without disabling Nagle's, small messages (heartbeats, position updates) are buffered for up to 40ms before being sent, which adds unacceptable latency to a real-time game. **SO_REUSEADDR**: on Linux and macOS, this allows rebinding to a port immediately after server restart; on Windows, the semantics are different (it allows multiple sockets to bind to the same port, which is a security concern). **TCP keepalive**: OS-level keepalive timers vary in default behavior and must be configured explicitly so the OS detects dead connections even if the application-level heartbeat (story 02) fails. **IPv6/dual-stack**: some platforms create dual-stack (IPv4+IPv6) sockets by default; others do not. The server should listen on both protocols where supported. These differences, if not handled, cause hard-to-debug platform-specific failures.

## Solution

### Socket configuration module

A `SocketConfig` struct encapsulates all platform-specific socket options. It is applied to every TCP socket (both server listener and client connections) immediately after creation.

```rust
use std::time::Duration;

pub struct SocketConfig {
    /// Disable Nagle's algorithm for lower latency. Default: true.
    pub tcp_nodelay: bool,
    /// Enable TCP keepalive. Default: true.
    pub keepalive_enabled: bool,
    /// Keepalive idle time — how long before the first keepalive probe. Default: 60s.
    pub keepalive_idle: Duration,
    /// Keepalive probe interval. Default: 10s.
    pub keepalive_interval: Duration,
    /// Number of keepalive probes before declaring connection dead. Default: 3.
    pub keepalive_retries: u32,
    /// Enable SO_REUSEADDR on server sockets. Default: true on Linux/macOS, false on Windows.
    pub reuse_addr: bool,
}

impl Default for SocketConfig {
    fn default() -> Self {
        Self {
            tcp_nodelay: true,
            keepalive_enabled: true,
            keepalive_idle: Duration::from_secs(60),
            keepalive_interval: Duration::from_secs(10),
            keepalive_retries: 3,
            reuse_addr: !cfg!(target_os = "windows"),
        }
    }
}
```

### Applying socket options

Socket options must be set on the raw socket before or immediately after binding/connecting. We use the `socket2` crate, which provides a cross-platform API for all socket options including keepalive.

```rust
use socket2::{SockRef, TcpKeepalive};
use tokio::net::{TcpListener, TcpStream};

/// Apply socket configuration to a connected TcpStream.
pub fn configure_stream(stream: &TcpStream, config: &SocketConfig) -> std::io::Result<()> {
    stream.set_nodelay(config.tcp_nodelay)?;

    if config.keepalive_enabled {
        let sock_ref = SockRef::from(stream);
        let keepalive = TcpKeepalive::new()
            .with_time(config.keepalive_idle)
            .with_interval(config.keepalive_interval);

        // Retries are supported on Linux and Windows but not macOS.
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        let keepalive = keepalive.with_retries(config.keepalive_retries);

        sock_ref.set_tcp_keepalive(&keepalive)?;
    }

    Ok(())
}

/// Create and configure a server TcpListener with proper socket options.
pub async fn create_listener(
    addr: std::net::SocketAddr,
    config: &SocketConfig,
) -> std::io::Result<TcpListener> {
    let socket = if addr.is_ipv6() {
        socket2::Socket::new(
            socket2::Domain::IPV6,
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )?
    } else {
        socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )?
    };

    // SO_REUSEADDR
    if config.reuse_addr {
        socket.set_reuse_address(true)?;
    }

    // For IPv6 sockets, disable IPV6_ONLY to enable dual-stack (accept both
    // IPv4 and IPv6 connections on a single socket). On Linux and macOS this
    // is the default; on Windows, IPV6_ONLY defaults to true, so we must
    // explicitly disable it.
    if addr.is_ipv6() {
        socket.set_only_v6(false)?;
    }

    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(128)?; // Backlog of 128

    // Convert socket2::Socket -> std::net::TcpListener -> tokio::net::TcpListener
    let std_listener: std::net::TcpListener = socket.into();
    TcpListener::from_std(std_listener)
}
```

### IPv4 and IPv6 support

The server defaults to binding on `[::]:7777` (IPv6 any-address with dual-stack), which accepts both IPv4 and IPv6 connections on platforms that support it. If dual-stack is not available (some minimal Linux configurations), the server falls back to binding on `0.0.0.0:7777` (IPv4 only) and optionally a separate `[::]:7777` (IPv6 only) listener.

```rust
/// Determine the best bind address for the server.
/// Prefers IPv6 dual-stack if available.
pub fn default_bind_address(port: u16) -> std::net::SocketAddr {
    // Try dual-stack IPv6 first
    std::net::SocketAddr::new(
        std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED),
        port,
    )
}

/// Fallback: bind IPv4 only.
pub fn ipv4_bind_address(port: u16) -> std::net::SocketAddr {
    std::net::SocketAddr::new(
        std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
        port,
    )
}
```

### Platform-specific notes

| Setting | Linux | Windows | macOS |
|---------|-------|---------|-------|
| `TCP_NODELAY` | Supported | Supported | Supported |
| `SO_REUSEADDR` | Allows quick rebind after restart | Allows multiple binds (dangerous) — disabled by default | Allows quick rebind after restart |
| `TCP_KEEPIDLE` | Supported via `setsockopt` | Supported via `SIO_KEEPALIVE_VALS` | Supported via `TCP_KEEPALIVE` |
| `TCP_KEEPINTVL` | Supported | Supported | Supported |
| `TCP_KEEPCNT` | Supported | Supported | **Not supported** (ignored) |
| `IPV6_V6ONLY` | Default: false (dual-stack) | Default: true (must set false) | Default: false (dual-stack) |

### Integration

This module is called by story 01 (TCP server) and story 02 (TCP client) to configure every socket immediately after creation. The `SocketConfig` is part of `ServerConfig` and `ClientConfig`.

## Outcome

A `platform.rs` module in `crates/nebula_net/src/` exporting `SocketConfig`, `configure_stream`, `create_listener`, `default_bind_address`, and `ipv4_bind_address`. All TCP sockets are configured with `TCP_NODELAY`, keepalive, and correct platform-specific options. The server supports dual-stack IPv4/IPv6 where the OS allows it. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

The server runs on Linux while the client runs on Windows (or vice versa). Cross-platform TCP connectivity is validated and works seamlessly.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | `1.49` (features: `net`) | `TcpListener`, `TcpStream` for async TCP |
| `socket2` | `0.5` | Cross-platform socket option configuration (`TcpKeepalive`, `SO_REUSEADDR`, `IPV6_V6ONLY`) |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpStream;
    use std::net::SocketAddr;

    #[tokio::test]
    async fn test_tcp_nodelay_is_set() {
        let config = SocketConfig::default();
        let listener = create_listener(
            "127.0.0.1:0".parse().unwrap(),
            &config,
        ).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client = TcpStream::connect(addr).await.unwrap();
        configure_stream(&client, &config).unwrap();

        assert!(
            client.nodelay().unwrap(),
            "TCP_NODELAY should be enabled"
        );
    }

    #[tokio::test]
    async fn test_connection_works_on_ipv4() {
        let config = SocketConfig::default();
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listener = create_listener(addr, &config).await.unwrap();
        let bound_addr = listener.local_addr().unwrap();

        let client = TcpStream::connect(bound_addr).await;
        assert!(client.is_ok(), "IPv4 connection should succeed");
    }

    #[tokio::test]
    async fn test_connection_works_on_ipv6() {
        let config = SocketConfig::default();
        let addr: SocketAddr = "[::1]:0".parse().unwrap();

        // IPv6 may not be available on all test environments.
        match create_listener(addr, &config).await {
            Ok(listener) => {
                let bound_addr = listener.local_addr().unwrap();
                let client = TcpStream::connect(bound_addr).await;
                assert!(client.is_ok(), "IPv6 connection should succeed");
            }
            Err(_) => {
                // IPv6 not available — skip test but don't fail.
                eprintln!("IPv6 not available, skipping test");
            }
        }
    }

    #[tokio::test]
    async fn test_keepalive_is_configured() {
        let config = SocketConfig {
            keepalive_enabled: true,
            keepalive_idle: Duration::from_secs(30),
            keepalive_interval: Duration::from_secs(5),
            keepalive_retries: 3,
            ..Default::default()
        };

        let listener = create_listener(
            "127.0.0.1:0".parse().unwrap(),
            &config,
        ).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client = TcpStream::connect(addr).await.unwrap();
        configure_stream(&client, &config).unwrap();

        // Verify keepalive is enabled by checking via socket2
        let sock_ref = SockRef::from(&client);
        let keepalive = sock_ref.keepalive().unwrap();
        assert!(keepalive, "Keepalive should be enabled");
    }

    #[tokio::test]
    async fn test_cross_platform_behavior_is_consistent() {
        // This test verifies that the default configuration applies without error
        // on the current platform. CI runs this on Linux, Windows, and macOS.
        let config = SocketConfig::default();

        let listener = create_listener(
            "127.0.0.1:0".parse().unwrap(),
            &config,
        ).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client = TcpStream::connect(addr).await.unwrap();
        let result = configure_stream(&client, &config);
        assert!(
            result.is_ok(),
            "Default socket configuration should succeed on all platforms: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn test_default_bind_address_is_ipv6() {
        let addr = default_bind_address(7777);
        assert!(addr.is_ipv6(), "Default bind address should be IPv6 for dual-stack");
        assert_eq!(addr.port(), 7777);
    }

    #[tokio::test]
    async fn test_ipv4_fallback_address() {
        let addr = ipv4_bind_address(7777);
        assert!(addr.is_ipv4(), "Fallback should be IPv4");
        assert_eq!(addr.port(), 7777);
    }

    #[tokio::test]
    async fn test_reuse_addr_platform_default() {
        let config = SocketConfig::default();
        if cfg!(target_os = "windows") {
            assert!(!config.reuse_addr, "SO_REUSEADDR should be disabled on Windows");
        } else {
            assert!(config.reuse_addr, "SO_REUSEADDR should be enabled on Linux/macOS");
        }
    }

    #[tokio::test]
    async fn test_nodelay_disabled_when_configured() {
        let config = SocketConfig {
            tcp_nodelay: false,
            ..Default::default()
        };

        let listener = create_listener(
            "127.0.0.1:0".parse().unwrap(),
            &SocketConfig::default(),
        ).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client = TcpStream::connect(addr).await.unwrap();
        configure_stream(&client, &config).unwrap();

        assert!(
            !client.nodelay().unwrap(),
            "TCP_NODELAY should be disabled when configured off"
        );
    }

    #[tokio::test]
    async fn test_server_listener_accepts_connections() {
        let config = SocketConfig::default();
        let listener = create_listener(
            "127.0.0.1:0".parse().unwrap(),
            &config,
        ).await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Connect multiple clients
        let c1 = TcpStream::connect(addr).await.unwrap();
        let c2 = TcpStream::connect(addr).await.unwrap();
        let c3 = TcpStream::connect(addr).await.unwrap();

        // Accept them all
        let (s1, _) = listener.accept().await.unwrap();
        let (s2, _) = listener.accept().await.unwrap();
        let (s3, _) = listener.accept().await.unwrap();

        // Verify all sockets are valid
        configure_stream(&s1, &config).unwrap();
        configure_stream(&s2, &config).unwrap();
        configure_stream(&s3, &config).unwrap();
    }
}
```
