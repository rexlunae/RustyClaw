//! Transport abstraction for gateway connections.
//!
//! This module provides a trait-based abstraction over the underlying
//! transport protocol (SSH, etc.), allowing the gateway to
//! handle connections uniformly regardless of how they arrive.
//!
//! ## Architecture
//!
//! The `Transport` trait defines the interface for sending and receiving
//! frames. Each transport implementation handles its own framing and
//! serialization, presenting a uniform async interface to the gateway.
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                       Gateway Core                          │
//! │  (auth, chat, tools, secrets, threads, tasks)              │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                    ┌────────┴────────┐
//!                    ▼                 ▼
//!                    ┌─────────────┐
//!                    │     SSH     │
//!                    │  Transport  │
//!                    └─────────────┘
//!                           │
//!                           ▼
//!                    ┌─────────────┐
//!                    │   TCP/SSH   │
//!                    │   Channel   │
//!                    └─────────────┘
//! ```
//!
//! ## Transport Types
//!
//! - **SSH**: Uses `russh` to accept SSH connections. Clients connect via
//!   standard SSH and frames are sent over the channel's stdin/stdout.
//!   Supports both standalone server mode and OpenSSH subsystem mode.

use super::protocol::{ClientFrame, ServerFrame};
use anyhow::Result;
use async_trait::async_trait;
use std::net::SocketAddr;

/// Information about the connected peer.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    /// Remote address of the peer (may be unknown for stdio transports).
    pub addr: Option<SocketAddr>,
    /// Username if authenticated via transport layer (e.g., SSH).
    pub username: Option<String>,
    /// Public key fingerprint if authenticated via SSH key.
    pub key_fingerprint: Option<String>,
    /// Transport type identifier.
    pub transport_type: TransportType,
}

/// The type of transport being used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportType {
    /// SSH channel (standalone server mode).
    Ssh,
    /// SSH subsystem via stdio (OpenSSH subsystem mode).
    SshSubsystem,
}

impl std::fmt::Display for TransportType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportType::Ssh => write!(f, "ssh"),
            TransportType::SshSubsystem => write!(f, "ssh-subsystem"),
        }
    }
}

/// A bidirectional transport for gateway communication.
///
/// This trait abstracts over different transport protocols, allowing the
/// gateway to handle connections uniformly. Each transport is responsible
/// for its own framing and serialization.
///
/// ## Splitting
///
/// Transports support splitting into separate read and write halves via
/// `into_split()`. This allows concurrent reading and writing, which is
/// essential for streaming responses while still accepting cancellation
/// or user input.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Get information about the connected peer.
    fn peer_info(&self) -> &PeerInfo;

    /// Receive the next frame from the client.
    ///
    /// Returns `None` if the connection is closed cleanly.
    /// Returns `Err` on protocol errors or unexpected disconnection.
    async fn recv(&mut self) -> Result<Option<ClientFrame>>;

    /// Send a frame to the client.
    async fn send(&mut self, frame: &ServerFrame) -> Result<()>;

    /// Close the transport gracefully.
    async fn close(&mut self) -> Result<()>;

    /// Split into separate read and write halves.
    ///
    /// This consumes the transport and returns two handles that can be
    /// used concurrently from different tasks.
    fn into_split(self: Box<Self>) -> (Box<dyn TransportReader>, Box<dyn TransportWriter>);
}

/// Read half of a split transport.
#[async_trait]
pub trait TransportReader: Send + Sync {
    /// Receive the next frame from the client.
    async fn recv(&mut self) -> Result<Option<ClientFrame>>;

    /// Get information about the connected peer.
    fn peer_info(&self) -> &PeerInfo;
}

/// Write half of a split transport.
#[async_trait]
pub trait TransportWriter: Send + Sync {
    /// Send a frame to the client.
    async fn send(&mut self, frame: &ServerFrame) -> Result<()>;

    /// Close the transport gracefully.
    async fn close(&mut self) -> Result<()>;
}

// ============================================================================
// Transport Acceptor Trait
// ============================================================================

/// A listener that accepts incoming transport connections.
///
/// This trait is implemented by both WebSocket and SSH servers,
/// allowing the gateway to accept connections from multiple sources.
#[async_trait]
pub trait TransportAcceptor: Send + Sync {
    /// Accept the next incoming connection.
    ///
    /// Returns a boxed transport that implements the `Transport` trait.
    async fn accept(&mut self) -> Result<Box<dyn Transport>>;

    /// Get the local address this acceptor is bound to.
    fn local_addr(&self) -> Result<SocketAddr>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transport_type_display() {
        assert_eq!(TransportType::Ssh.to_string(), "ssh");
        assert_eq!(TransportType::SshSubsystem.to_string(), "ssh-subsystem");
    }

    #[test]
    fn test_peer_info_default() {
        let info = PeerInfo {
            addr: None,
            username: Some("test".to_string()),
            key_fingerprint: Some("SHA256:abc123".to_string()),
            transport_type: TransportType::Ssh,
        };
        assert_eq!(info.username.as_deref(), Some("test"));
        assert_eq!(info.transport_type, TransportType::Ssh);
    }
}
