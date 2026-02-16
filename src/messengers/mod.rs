//! Messenger implementations for various chat platforms.
//!
//! Each messenger implements the `Messenger` trait and can be enabled
//! via feature flags in Cargo.toml.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ── Core types ──────────────────────────────────────────────────────────────

/// Represents a message in the messenger system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub sender: String,
    pub content: String,
    pub timestamp: i64,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub media: Option<Vec<MediaAttachment>>,
}

/// Media attachment in a message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaAttachment {
    pub url: Option<String>,
    pub path: Option<String>,
    pub mime_type: Option<String>,
    pub filename: Option<String>,
}

/// Options for sending a message
#[derive(Debug, Default)]
pub struct SendOptions<'a> {
    pub recipient: &'a str,
    pub content: &'a str,
    pub reply_to: Option<&'a str>,
    pub silent: bool,
    pub media: Option<&'a str>,
}

// ── Messenger trait ─────────────────────────────────────────────────────────

/// Trait for messenger implementations (OpenClaw compatible)
#[async_trait]
pub trait Messenger: Send + Sync {
    /// Get the messenger name
    fn name(&self) -> &str;

    /// Get the messenger type (telegram, discord, signal, matrix, etc.)
    fn messenger_type(&self) -> &str;

    /// Initialize the messenger (connect, authenticate, etc.)
    async fn initialize(&mut self) -> Result<()>;

    /// Send a message to a recipient/channel
    async fn send_message(&self, recipient: &str, content: &str) -> Result<String>;

    /// Send a message with additional options
    async fn send_message_with_options(&self, opts: SendOptions<'_>) -> Result<String> {
        // Default implementation ignores options
        self.send_message(opts.recipient, opts.content).await
    }

    /// Receive pending messages (non-blocking poll)
    async fn receive_messages(&self) -> Result<Vec<Message>>;

    /// Check if the messenger is connected
    fn is_connected(&self) -> bool;

    /// Disconnect the messenger
    async fn disconnect(&mut self) -> Result<()>;

    /// Set typing indicator status (optional, defaults to no-op)
    async fn set_typing(&self, _channel: &str, _typing: bool) -> Result<()> {
        // Default implementation does nothing
        // Platforms that support typing indicators should override this
        Ok(())
    }

    /// Update presence status (optional, defaults to no-op)
    async fn set_presence(&self, _status: &str) -> Result<()> {
        // Default implementation does nothing
        // Platforms that support presence should override this
        Ok(())
    }
}

// ── Messenger manager ───────────────────────────────────────────────────────

/// Manager for multiple messengers
pub struct MessengerManager {
    messengers: Vec<Box<dyn Messenger>>,
}

impl MessengerManager {
    pub fn new() -> Self {
        Self {
            messengers: Vec::new(),
        }
    }

    /// Add a messenger to the manager
    pub fn add_messenger(&mut self, messenger: Box<dyn Messenger>) {
        self.messengers.push(messenger);
    }

    /// Initialize all messengers
    pub async fn initialize_all(&mut self) -> Result<()> {
        for messenger in &mut self.messengers {
            messenger.initialize().await?;
        }
        Ok(())
    }

    /// Get all messengers
    pub fn get_messengers(&self) -> &[Box<dyn Messenger>] {
        &self.messengers
    }

    /// Get a messenger by name
    pub fn get_messenger(&self, name: &str) -> Option<&dyn Messenger> {
        self.messengers
            .iter()
            .find(|m| m.name() == name)
            .map(|b| &**b)
    }

    /// Get a messenger by type
    pub fn get_messenger_by_type(&self, msg_type: &str) -> Option<&dyn Messenger> {
        self.messengers
            .iter()
            .find(|m| m.messenger_type() == msg_type)
            .map(|b| &**b)
    }

    /// Disconnect all messengers
    pub async fn disconnect_all(&mut self) -> Result<()> {
        for messenger in &mut self.messengers {
            messenger.disconnect().await?;
        }
        Ok(())
    }
}

impl Default for MessengerManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── Built-in messengers ─────────────────────────────────────────────────────

mod webhook;
mod console;
mod discord;
mod telegram;
mod gmail;
mod slack;

pub use webhook::WebhookMessenger;
pub use console::ConsoleMessenger;
pub use discord::DiscordMessenger;
pub use telegram::TelegramMessenger;
pub use gmail::{GmailMessenger, GmailConfig};
pub use slack::{SlackMessenger, SlackConfig};

// ── Optional messengers (feature-gated) ─────────────────────────────────────

#[cfg(feature = "matrix")]
mod matrix;
#[cfg(feature = "matrix")]
pub use matrix::MatrixMessenger;

#[cfg(feature = "signal")]
mod signal;
#[cfg(feature = "signal")]
pub use signal::SignalMessenger;
