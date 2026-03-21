//! Transport abstraction for gateway connections.
//!
//! This module provides a trait-based abstraction over the underlying
//! transport protocol (WebSocket, SSH, etc.), allowing the gateway to
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
//!            ┌─────────────┐   ┌─────────────┐
//!            │  WebSocket  │   │     SSH     │
//!            │  Transport  │   │  Transport  │
//!            └─────────────┘   └─────────────┘
//!                    │                 │
//!                    ▼                 ▼
//!            ┌─────────────┐   ┌─────────────┐
//!            │   TCP/TLS   │   │   TCP/SSH   │
//!            │   Socket    │   │   Channel   │
//!            └─────────────┘   └─────────────┘
//! ```
//!
//! ## Transport Types
//!
//! - **WebSocket**: The default transport, using `tokio-tungstenite`.
//!   Frames are serialized with bincode and sent as binary WebSocket messages.
//!
//! - **SSH**: Uses `russh` to accept SSH connections. Clients connect via
//!   standard SSH and frames are sent over the channel's stdin/stdout.
//!   Supports both standalone server mode and OpenSSH subsystem mode.

use super::protocol::{ClientFrame, ServerFrame, deserialize_frame, serialize_frame};
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
    /// WebSocket over TCP/TLS.
    WebSocket,
    /// SSH channel (standalone server mode).
    Ssh,
    /// SSH subsystem via stdio (OpenSSH subsystem mode).
    SshSubsystem,
}

impl std::fmt::Display for TransportType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportType::WebSocket => write!(f, "websocket"),
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
// WebSocket Transport Implementation
// ============================================================================

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

/// WebSocket transport wrapper.
///
/// This wraps a `WebSocketStream` and implements the `Transport` trait,
/// sending and receiving frames as binary WebSocket messages.
pub struct WebSocketTransport<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    stream: WebSocketStream<S>,
    peer_info: PeerInfo,
}

impl<S> WebSocketTransport<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    /// Create a new WebSocket transport from an accepted connection.
    pub fn new(stream: WebSocketStream<S>, peer_addr: Option<SocketAddr>) -> Self {
        Self {
            stream,
            peer_info: PeerInfo {
                addr: peer_addr,
                username: None,
                key_fingerprint: None,
                transport_type: TransportType::WebSocket,
            },
        }
    }
}

#[async_trait]
impl<S> Transport for WebSocketTransport<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
{
    fn peer_info(&self) -> &PeerInfo {
        &self.peer_info
    }

    async fn recv(&mut self) -> Result<Option<ClientFrame>> {
        loop {
            match self.stream.next().await {
                Some(Ok(Message::Binary(data))) => {
                    return deserialize_frame(&data)
                        .map(Some)
                        .map_err(|e| anyhow::anyhow!(e));
                }
                Some(Ok(Message::Close(_))) => {
                    return Ok(None);
                }
                Some(Ok(Message::Ping(data))) => {
                    // Respond to pings automatically
                    self.stream.send(Message::Pong(data)).await?;
                }
                Some(Ok(Message::Pong(_))) => {
                    // Ignore pongs
                }
                Some(Ok(Message::Text(_))) => {
                    anyhow::bail!("Text frames not supported; use binary");
                }
                Some(Ok(Message::Frame(_))) => {
                    // Raw frame, shouldn't happen with normal usage
                }
                Some(Err(e)) => {
                    return Err(e.into());
                }
                None => {
                    return Ok(None);
                }
            }
        }
    }

    async fn send(&mut self, frame: &ServerFrame) -> Result<()> {
        let data = serialize_frame(frame).map_err(|e| anyhow::anyhow!(e))?;
        self.stream.send(Message::Binary(data.into())).await?;
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        self.stream.close(None).await?;
        Ok(())
    }

    fn into_split(self: Box<Self>) -> (Box<dyn TransportReader>, Box<dyn TransportWriter>) {
        let (write, read) = self.stream.split();
        let peer_info = self.peer_info.clone();
        (
            Box::new(WebSocketReader {
                stream: read,
                peer_info: peer_info.clone(),
            }),
            Box::new(WebSocketWriter {
                stream: write,
                _peer_info: peer_info,
            }),
        )
    }
}

/// Read half of a WebSocket transport.
pub struct WebSocketReader<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    stream: SplitStream<WebSocketStream<S>>,
    peer_info: PeerInfo,
}

#[async_trait]
impl<S> TransportReader for WebSocketReader<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
{
    async fn recv(&mut self) -> Result<Option<ClientFrame>> {
        loop {
            match self.stream.next().await {
                Some(Ok(Message::Binary(data))) => {
                    return deserialize_frame(&data)
                        .map(Some)
                        .map_err(|e| anyhow::anyhow!(e));
                }
                Some(Ok(Message::Close(_))) => {
                    return Ok(None);
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Text(_) | Message::Frame(_))) => {
                    // Skip non-binary messages in reader (pings handled by writer task)
                }
                Some(Err(e)) => {
                    return Err(e.into());
                }
                None => {
                    return Ok(None);
                }
            }
        }
    }

    fn peer_info(&self) -> &PeerInfo {
        &self.peer_info
    }
}

/// Write half of a WebSocket transport.
pub struct WebSocketWriter<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    stream: SplitSink<WebSocketStream<S>, Message>,
    _peer_info: PeerInfo,
}

#[async_trait]
impl<S> TransportWriter for WebSocketWriter<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
{
    async fn send(&mut self, frame: &ServerFrame) -> Result<()> {
        let data = serialize_frame(frame).map_err(|e| anyhow::anyhow!(e))?;
        self.stream.send(Message::Binary(data.into())).await?;
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        self.stream.close().await?;
        Ok(())
    }
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
        assert_eq!(TransportType::WebSocket.to_string(), "websocket");
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
