//! Cross-platform TCP socket configuration.
//!
//! Provides [`SocketConfig`] to encapsulate platform-specific socket options
//! (TCP_NODELAY, keepalive, SO_REUSEADDR, dual-stack IPv6) and helper functions
//! to apply them consistently across Linux, Windows, and macOS.

use std::time::Duration;

use socket2::{SockRef, TcpKeepalive};
use tokio::net::{TcpListener, TcpStream};

/// Platform-specific TCP socket configuration applied to every connection.
#[derive(Debug, Clone)]
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
    /// Enable `SO_REUSEADDR` on server sockets. Default: true on Linux/macOS, false on Windows.
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

/// Apply socket configuration to a connected [`TcpStream`].
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

/// Create and configure a server [`TcpListener`] with proper socket options.
///
/// Sets `SO_REUSEADDR`, dual-stack IPv6 (when binding to an IPv6 address),
/// and non-blocking mode before binding.
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

    if config.reuse_addr {
        socket.set_reuse_address(true)?;
    }

    // For IPv6 sockets, disable IPV6_ONLY to enable dual-stack (accept both
    // IPv4 and IPv6 connections on a single socket).
    if addr.is_ipv6() {
        socket.set_only_v6(false)?;
    }

    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(128)?;

    let std_listener: std::net::TcpListener = socket.into();
    TcpListener::from_std(std_listener)
}

/// Determine the best bind address for the server.
///
/// Prefers IPv6 dual-stack (`[::]`) if available, which accepts both IPv4 and
/// IPv6 connections on platforms that support it.
pub fn default_bind_address(port: u16) -> std::net::SocketAddr {
    std::net::SocketAddr::new(std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED), port)
}

/// Fallback: bind IPv4 only (`0.0.0.0`).
pub fn ipv4_bind_address(port: u16) -> std::net::SocketAddr {
    std::net::SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tcp_nodelay_is_set() {
        let config = SocketConfig::default();
        let listener = create_listener("127.0.0.1:0".parse().unwrap(), &config)
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();

        let client = TcpStream::connect(addr).await.unwrap();
        configure_stream(&client, &config).unwrap();

        assert!(client.nodelay().unwrap(), "TCP_NODELAY should be enabled");
    }

    #[tokio::test]
    async fn test_connection_works_on_ipv4() {
        let config = SocketConfig::default();
        let addr: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listener = create_listener(addr, &config).await.unwrap();
        let bound_addr = listener.local_addr().unwrap();

        let client = TcpStream::connect(bound_addr).await;
        assert!(client.is_ok(), "IPv4 connection should succeed");
    }

    #[tokio::test]
    async fn test_connection_works_on_ipv6() {
        let config = SocketConfig::default();
        let addr: std::net::SocketAddr = "[::1]:0".parse().unwrap();

        match create_listener(addr, &config).await {
            Ok(listener) => {
                let bound_addr = listener.local_addr().unwrap();
                let client = TcpStream::connect(bound_addr).await;
                assert!(client.is_ok(), "IPv6 connection should succeed");
            }
            Err(_) => {
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

        let listener = create_listener("127.0.0.1:0".parse().unwrap(), &config)
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();

        let client = TcpStream::connect(addr).await.unwrap();
        configure_stream(&client, &config).unwrap();

        let sock_ref = SockRef::from(&client);
        let keepalive = sock_ref.keepalive().unwrap();
        assert!(keepalive, "Keepalive should be enabled");
    }

    #[tokio::test]
    async fn test_cross_platform_behavior_is_consistent() {
        let config = SocketConfig::default();

        let listener = create_listener("127.0.0.1:0".parse().unwrap(), &config)
            .await
            .unwrap();
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
        assert!(
            addr.is_ipv6(),
            "Default bind address should be IPv6 for dual-stack"
        );
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
            assert!(
                !config.reuse_addr,
                "SO_REUSEADDR should be disabled on Windows"
            );
        } else {
            assert!(
                config.reuse_addr,
                "SO_REUSEADDR should be enabled on Linux/macOS"
            );
        }
    }

    #[tokio::test]
    async fn test_nodelay_disabled_when_configured() {
        let config = SocketConfig {
            tcp_nodelay: false,
            ..Default::default()
        };

        let listener = create_listener("127.0.0.1:0".parse().unwrap(), &SocketConfig::default())
            .await
            .unwrap();
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
        let listener = create_listener("127.0.0.1:0".parse().unwrap(), &config)
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();

        let _c1 = TcpStream::connect(addr).await.unwrap();
        let _c2 = TcpStream::connect(addr).await.unwrap();
        let _c3 = TcpStream::connect(addr).await.unwrap();

        let (s1, _) = listener.accept().await.unwrap();
        let (s2, _) = listener.accept().await.unwrap();
        let (s3, _) = listener.accept().await.unwrap();

        configure_stream(&s1, &config).unwrap();
        configure_stream(&s2, &config).unwrap();
        configure_stream(&s3, &config).unwrap();
    }
}
