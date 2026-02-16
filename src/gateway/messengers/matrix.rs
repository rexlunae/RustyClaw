//! Matrix messenger integration
//!
//! Provides Matrix protocol support for RustyClaw, allowing the assistant
//! to be accessible through Matrix homeservers (Element, matrix.org, etc.).

use super::{Messenger, MessengerEvent, MessengerMessage};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

// Note: This is a placeholder implementation. Full Matrix integration
// would use the matrix-sdk crate which is already in dependencies.
// For now, this demonstrates the structure and configuration.

/// Matrix messenger configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixConfig {
    /// Matrix homeserver URL (e.g., "https://matrix.org")
    pub homeserver_url: String,
    /// Bot username (e.g., "@rustyclaw:matrix.org")
    pub username: String,
    /// Bot password or access token
    pub password: String,
    /// Device ID (optional, for E2EE)
    pub device_id: Option<String>,
    /// Device display name
    pub device_name: Option<String>,
}

impl Default for MatrixConfig {
    fn default() -> Self {
        Self {
            homeserver_url: "https://matrix.org".to_string(),
            username: String::new(),
            password: String::new(),
            device_id: None,
            device_name: Some("RustyClaw Bot".to_string()),
        }
    }
}

/// Matrix messenger implementation
pub struct MatrixMessenger {
    config: MatrixConfig,
    event_tx: mpsc::UnboundedSender<MessengerEvent>,
    running: Arc<RwLock<bool>>,
}

impl MatrixMessenger {
    /// Create a new Matrix messenger
    pub fn new(config: MatrixConfig, event_tx: mpsc::UnboundedSender<MessengerEvent>) -> Self {
        Self {
            config,
            event_tx,
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// Login to Matrix homeserver
    async fn login(&self) -> Result<()> {
        // In a full implementation, this would use matrix_sdk::Client
        // to log in to the homeserver and obtain an access token

        eprintln!("[matrix] Logging in to {}", self.config.homeserver_url);
        eprintln!("[matrix] Username: {}", self.config.username);

        // Placeholder: In reality, would use:
        // let client = matrix_sdk::Client::new(homeserver_url).await?;
        // client.login(username, password, device_id, display_name).await?;

        Ok(())
    }

    /// Start sync loop to receive events
    async fn start_sync(&self) -> Result<()> {
        *self.running.write().await = true;

        let event_tx = self.event_tx.clone();
        let running = self.running.clone();

        tokio::spawn(async move {
            eprintln!("[matrix] Starting sync loop...");

            while *running.read().await {
                // In a full implementation, this would:
                // 1. Call client.sync().await to get new events
                // 2. Process timeline events (messages, reactions, etc.)
                // 3. Forward to event_tx

                // Placeholder sync
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }

            eprintln!("[matrix] Sync loop stopped");
        });

        Ok(())
    }

    /// Send a message to Matrix room
    async fn post_message(&self, room_id: &str, content: &str) -> Result<()> {
        // In a full implementation, this would use:
        // let room = client.get_room(room_id).context("Room not found")?;
        // room.send(RoomMessageEventContent::text_plain(content)).await?;

        eprintln!("[matrix] Sending message to room {}: {}", room_id, content);

        Ok(())
    }

    /// Send a formatted message (HTML + plain text)
    pub async fn send_formatted_message(&self, room_id: &str, plain: &str, html: &str) -> Result<()> {
        // In a full implementation:
        // let room = client.get_room(room_id).context("Room not found")?;
        // let content = RoomMessageEventContent::text_html(plain, html);
        // room.send(content).await?;

        eprintln!("[matrix] Sending formatted message to room {}", room_id);
        eprintln!("[matrix] Plain: {}", plain);
        eprintln!("[matrix] HTML: {}", html);

        Ok(())
    }

    /// Join a Matrix room
    pub async fn join_room(&self, room_id_or_alias: &str) -> Result<()> {
        // In a full implementation:
        // client.join_room_by_id_or_alias(room_id_or_alias, &[]).await?;

        eprintln!("[matrix] Joining room: {}", room_id_or_alias);

        Ok(())
    }

    /// Leave a Matrix room
    pub async fn leave_room(&self, room_id: &str) -> Result<()> {
        // In a full implementation:
        // let room = client.get_room(room_id).context("Room not found")?;
        // room.leave().await?;

        eprintln!("[matrix] Leaving room: {}", room_id);

        Ok(())
    }

    /// Add reaction to a message
    pub async fn add_reaction(&self, room_id: &str, event_id: &str, emoji: &str) -> Result<()> {
        // In a full implementation:
        // let room = client.get_room(room_id).context("Room not found")?;
        // room.send_reaction(event_id, emoji).await?;

        eprintln!("[matrix] Adding reaction {} to message {} in room {}", emoji, event_id, room_id);

        Ok(())
    }
}

#[async_trait]
impl Messenger for MatrixMessenger {
    async fn start(&self) -> Result<()> {
        self.login().await?;
        self.start_sync().await?;
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        *self.running.write().await = false;
        // In a full implementation, would also call client.logout().await
        Ok(())
    }

    async fn send_message(&self, channel_id: &str, message: &str) -> Result<()> {
        self.post_message(channel_id, message).await
    }

    fn platform_name(&self) -> &str {
        "matrix"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matrix_config_default() {
        let config = MatrixConfig::default();
        assert_eq!(config.homeserver_url, "https://matrix.org");
        assert_eq!(config.device_name, Some("RustyClaw Bot".to_string()));
    }

    #[tokio::test]
    async fn test_matrix_messenger_creation() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let config = MatrixConfig::default();
        let messenger = MatrixMessenger::new(config, tx);
        assert_eq!(messenger.platform_name(), "matrix");
    }
}
