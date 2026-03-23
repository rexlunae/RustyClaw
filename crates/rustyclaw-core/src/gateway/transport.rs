use std::fmt;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use thiserror::Error;
use uuid::Uuid;

#[derive(Error, Debug)]
pub enum TransportError {
    #[error("Connection closed")]
    ConnectionClosed,
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
    #[error("Protocol error: {0}")]
    ProtocolError(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Transport error: {0}")]
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConnectionInfo {
    WebSocket { 
        remote_addr: String,
        user_agent: Option<String>,
    },
    Ssh { 
        remote_addr: String,
        username: String,
        public_key_fingerprint: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct TransportMessage {
    pub content: Vec<u8>,
    pub message_type: MessageType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MessageType {
    Text,
    Binary,
    Close,
    Ping,
    Pong,
}

/// Transport abstraction for different connection types (WebSocket, SSH, etc)
#[async_trait]
pub trait Transport: Send + Sync {
    /// Get connection information
    fn connection_info(&self) -> &ConnectionInfo;

    /// Send a message through the transport
    async fn send(&mut self, message: TransportMessage) -> Result<(), TransportError>;

    /// Receive the next message from the transport
    async fn receive(&mut self) -> Result<Option<TransportMessage>, TransportError>;

    /// Close the transport connection
    async fn close(&mut self) -> Result<(), TransportError>;

    /// Check if the transport is still connected
    fn is_connected(&self) -> bool;
}

/// Factory for creating transport instances
#[async_trait]
pub trait TransportFactory: Send + Sync {
    type Transport: Transport + 'static;
    type Config: Clone + Send + Sync;

    /// Create and start a transport listener
    async fn listen(&self, config: Self::Config) -> Result<TransportListener<Self::Transport>, TransportError>;
}

/// Listener for incoming transport connections
pub struct TransportListener<T: Transport> {
    receiver: mpsc::UnboundedReceiver<Result<T, TransportError>>,
    _handle: tokio::task::JoinHandle<()>,
}

impl<T: Transport> TransportListener<T> {
    pub fn new(
        receiver: mpsc::UnboundedReceiver<Result<T, TransportError>>,
        handle: tokio::task::JoinHandle<()>,
    ) -> Self {
        Self {
            receiver,
            _handle: handle,
        }
    }

    pub async fn accept(&mut self) -> Option<Result<T, TransportError>> {
        self.receiver.recv().await
    }
}

/// Helper to convert between MessageType and text/binary
impl MessageType {
    pub fn is_text(&self) -> bool {
        matches!(self, MessageType::Text)
    }

    pub fn is_binary(&self) -> bool {
        matches!(self, MessageType::Binary)
    }

    pub fn is_close(&self) -> bool {
        matches!(self, MessageType::Close)
    }
}

impl fmt::Display for MessageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MessageType::Text => write!(f, "text"),
            MessageType::Binary => write!(f, "binary"),
            MessageType::Close => write!(f, "close"),
            MessageType::Ping => write!(f, "ping"),
            MessageType::Pong => write!(f, "pong"),
        }
    }
}