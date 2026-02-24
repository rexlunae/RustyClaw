//! Messenger implementations for various chat platforms.
//!
//! **DEPRECATION NOTICE**: Native messenger integrations are being phased out
//! in favor of skill-based messaging via [Beeper](https://www.beeper.com) and
//! the `claw-me-maybe` skill from ClawHub. The skill approach:
//! - Requires no recompilation for new platforms
//! - Handles 15+ platforms through one API
//! - Is the recommended path in `rustyclaw onboard`
//!
//! The native messengers below are retained for backwards compatibility but
//! are largely untested. New development should focus on skill-based approaches.
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

pub use webhook::WebhookMessenger;
pub use console::ConsoleMessenger;
pub use discord::DiscordMessenger;
pub use telegram::TelegramMessenger;

// ── Additional messengers ───────────────────────────────────────────────────
// Note: These are deprecated in favor of skill-based messaging (see module docs).
// They remain compiled-in for backwards compatibility.

mod matrix;
pub use matrix::MatrixMessenger;

// Signal messenger removed — was incomplete and never had its dependencies (presage) added.
// Use the signal-messenger-standalone skill or claw-me-maybe for Signal integration.

mod whatsapp;
pub use whatsapp::WhatsAppMessenger;
