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

use super::protocol::{CONTROL_STREAM_ID, ClientFrame, ServerFrame, WireFrame};
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
    async fn recv(&mut self) -> Result<Option<WireFrame<ClientFrame>>>;

    /// Send a frame to the client.
    async fn send(&mut self, frame: &ServerFrame) -> Result<()> {
        self.send_on_stream(CONTROL_STREAM_ID, frame).await
    }

    /// Send a frame to the client on a logical stream.
    async fn send_on_stream(&mut self, stream_id: u64, frame: &ServerFrame) -> Result<()>;

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
    async fn recv(&mut self) -> Result<Option<WireFrame<ClientFrame>>>;

    /// Get information about the connected peer.
    fn peer_info(&self) -> &PeerInfo;
}

/// Write half of a split transport.
#[async_trait]
pub trait TransportWriter: Send + Sync {
    /// Send a frame to the client.
    async fn send(&mut self, frame: &ServerFrame) -> Result<()> {
        self.send_on_stream(CONTROL_STREAM_ID, frame).await
    }

    /// Send a frame to the client on a logical stream.
    async fn send_on_stream(&mut self, stream_id: u64, frame: &ServerFrame) -> Result<()>;

    /// Close the transport gracefully.
    async fn close(&mut self) -> Result<()>;
}

/// A write handed to the connection's writer task.
pub enum Outbound {
    Frame {
        stream_id: u64,
        frame: Box<ServerFrame>,
    },
    Close,
}

/// Writer that hands frames to the connection's writer task instead of
/// touching the transport.
///
/// The transport permits one writer, which historically meant the connection
/// loop owned it and everything wanting to answer the client had to reach that
/// loop first. Read-only queries then waited on whatever the loop was doing —
/// a model call, a tool, a lock — and a client asking for a thread's history
/// could sit unanswered indefinitely, showing an empty transcript.
///
/// Cloning this hands out another mouth for the same queue, so a task that
/// needs to answer without the loop can. Serialisation moves to the queue,
/// which is what keeps a reply from interleaving into a streaming turn's
/// frames: order is the order things were enqueued, and there is still exactly
/// one task writing.
#[derive(Clone)]
pub struct QueuedWriter {
    tx: tokio::sync::mpsc::Sender<Outbound>,
}

impl QueuedWriter {
    pub fn new(tx: tokio::sync::mpsc::Sender<Outbound>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl TransportWriter for QueuedWriter {
    async fn send_on_stream(&mut self, stream_id: u64, frame: &ServerFrame) -> Result<()> {
        self.tx
            .send(Outbound::Frame {
                stream_id,
                frame: Box::new(frame.clone()),
            })
            .await
            .map_err(|_| anyhow::anyhow!("connection writer has gone away"))
    }

    async fn close(&mut self) -> Result<()> {
        // Through the queue rather than straight to the transport, so frames
        // already enqueued still go out ahead of the close.
        self.tx
            .send(Outbound::Close)
            .await
            .map_err(|_| anyhow::anyhow!("connection writer has gone away"))
    }
}

/// Drain `rx` onto `writer` until the queue closes or the transport fails.
///
/// The single point where frames meet the transport: every producer holds a
/// [`QueuedWriter`], so this is what makes concurrent senders safe.
pub async fn drive_writer(
    mut writer: Box<dyn TransportWriter>,
    mut rx: tokio::sync::mpsc::Receiver<Outbound>,
) {
    while let Some(outbound) = rx.recv().await {
        match outbound {
            Outbound::Frame { stream_id, frame } => {
                if writer.send_on_stream(stream_id, &frame).await.is_err() {
                    break;
                }
            }
            Outbound::Close => {
                let _ = writer.close().await;
                break;
            }
        }
    }
}

/// Writer adapter that pins all writes to one logical stream.
pub struct ScopedTransportWriter<'a> {
    inner: &'a mut dyn TransportWriter,
    stream_id: u64,
}

impl<'a> ScopedTransportWriter<'a> {
    pub fn new(inner: &'a mut dyn TransportWriter, stream_id: u64) -> Self {
        Self { inner, stream_id }
    }
}

#[async_trait]
impl<'a> TransportWriter for ScopedTransportWriter<'a> {
    async fn send_on_stream(&mut self, _stream_id: u64, frame: &ServerFrame) -> Result<()> {
        self.inner.send_on_stream(self.stream_id, frame).await
    }

    async fn close(&mut self) -> Result<()> {
        self.inner.close().await
    }
}

// ============================================================================
// Transport Acceptor Trait
// ============================================================================

/// A listener that accepts incoming transport connections.
///
/// Implemented by the SSH server today; the abstraction allows the gateway
/// to accept connections from additional transports (e.g. WebSocket) later.
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
    use crate::gateway::{ServerFrameType, ServerPayload};

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

    /// Records what actually reached the transport, in order.
    struct RecordingWriter {
        seen: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl TransportWriter for RecordingWriter {
        async fn send_on_stream(&mut self, stream_id: u64, _frame: &ServerFrame) -> Result<()> {
            self.seen
                .lock()
                .expect("recorder poisoned")
                .push(format!("frame:{stream_id}"));
            Ok(())
        }

        async fn close(&mut self) -> Result<()> {
            self.seen
                .lock()
                .expect("recorder poisoned")
                .push("close".to_string());
            Ok(())
        }
    }

    fn empty_frame() -> ServerFrame {
        ServerFrame {
            frame_type: ServerFrameType::Status,
            payload: ServerPayload::Empty,
        }
    }

    /// Concurrent producers reach the transport in the order they enqueued.
    ///
    /// This is what lets a task other than the connection loop answer the
    /// client: the transport still has exactly one writer, so a reply cannot
    /// land in the middle of another task's frame.
    #[tokio::test]
    async fn queued_writes_reach_the_transport_in_enqueue_order() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (tx, rx) = tokio::sync::mpsc::channel::<Outbound>(64);
        let driver = tokio::spawn(drive_writer(
            Box::new(RecordingWriter { seen: seen.clone() }),
            rx,
        ));

        // Two independent handles, as the connection loop and the reader task
        // hold — interleaved deliberately.
        let mut loop_writer = QueuedWriter::new(tx.clone());
        let mut reader_writer = QueuedWriter::new(tx.clone());
        for stream_id in 0..4u64 {
            let writer: &mut dyn TransportWriter = if stream_id % 2 == 0 {
                &mut loop_writer
            } else {
                &mut reader_writer
            };
            writer
                .send_on_stream(stream_id, &empty_frame())
                .await
                .expect("queued send should succeed");
        }

        drop(loop_writer);
        drop(reader_writer);
        drop(tx);
        driver.await.expect("writer task should finish");

        let seen = seen.lock().expect("recorder poisoned").clone();
        assert_eq!(
            seen,
            vec!["frame:0", "frame:1", "frame:2", "frame:3"],
            "frames must reach the transport in the order they were enqueued"
        );
    }

    /// `close` travels the queue, so frames enqueued before it still go out.
    ///
    /// Closing the transport directly would cut off whatever was already
    /// waiting — including a connection's final close-out.
    #[tokio::test]
    async fn close_is_ordered_behind_pending_frames() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (tx, rx) = tokio::sync::mpsc::channel::<Outbound>(64);
        let driver = tokio::spawn(drive_writer(
            Box::new(RecordingWriter { seen: seen.clone() }),
            rx,
        ));

        let mut writer = QueuedWriter::new(tx.clone());
        writer
            .send_on_stream(7, &empty_frame())
            .await
            .expect("queued send should succeed");
        writer.close().await.expect("queued close should succeed");

        drop(writer);
        drop(tx);
        driver.await.expect("writer task should finish");

        let seen = seen.lock().expect("recorder poisoned").clone();
        assert_eq!(seen, vec!["frame:7", "close"]);
    }
}
