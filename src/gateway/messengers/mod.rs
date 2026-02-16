//! Messenger channel integrations
//!
//! This module provides integrations with various messaging platforms,
//! allowing RustyClaw to be accessible through multiple channels.

#[cfg(feature = "messenger-slack")]
pub mod slack;

#[cfg(feature = "messenger-discord")]
pub mod discord;

#[cfg(feature = "messenger-telegram")]
pub mod telegram;

#[cfg(feature = "matrix")]
pub mod matrix;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Common trait for all messenger integrations
#[async_trait]
pub trait Messenger: Send + Sync {
    /// Start the messenger bot/webhook server
    async fn start(&self) -> Result<()>;

    /// Stop the messenger bot
    async fn stop(&self) -> Result<()>;

    /// Send a message to a specific channel/chat
    async fn send_message(&self, channel_id: &str, message: &str) -> Result<()>;

    /// Get the messenger platform name
    fn platform_name(&self) -> &str;
}

/// Common message structure across all messengers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessengerMessage {
    pub platform: String,
    pub channel_id: String,
    pub user_id: String,
    pub user_name: String,
    pub text: String,
    pub timestamp: i64,
    pub thread_id: Option<String>,
}

/// Messenger event types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessengerEvent {
    Message(MessengerMessage),
    Reaction { message_id: String, emoji: String, user_id: String },
    Join { channel_id: String, user_id: String },
    Leave { channel_id: String, user_id: String },
}
