//! Discord messenger integration
//!
//! Provides Discord bot functionality for RustyClaw, allowing the assistant
//! to be accessible through Discord servers.

use super::{Messenger, MessengerEvent, MessengerMessage};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

/// Discord messenger configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordConfig {
    /// Discord bot token
    pub bot_token: String,
    /// Application ID
    pub application_id: String,
    /// Command prefix (e.g., "!")
    pub command_prefix: Option<String>,
    /// Whether to respond to all messages or only mentions/commands
    pub respond_to_all: bool,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            application_id: String::new(),
            command_prefix: Some("!".to_string()),
            respond_to_all: false,
        }
    }
}

/// Discord messenger implementation
pub struct DiscordMessenger {
    config: DiscordConfig,
    client: reqwest::Client,
    event_tx: mpsc::UnboundedSender<MessengerEvent>,
    running: Arc<RwLock<bool>>,
}

impl DiscordMessenger {
    /// Create a new Discord messenger
    pub fn new(config: DiscordConfig, event_tx: mpsc::UnboundedSender<MessengerEvent>) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            event_tx,
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// Start Discord Gateway connection
    async fn connect_gateway(&self) -> Result<()> {
        let gateway_url = self.get_gateway_url().await?;
        eprintln!("[discord] Connecting to gateway: {}", gateway_url);

        // In a full implementation, this would:
        // 1. Connect to WebSocket gateway
        // 2. Authenticate with bot token
        // 3. Start heartbeat
        // 4. Listen for events

        *self.running.write().await = true;

        // TODO: Implement full Discord Gateway WebSocket client
        // For now, this is a placeholder that demonstrates the structure

        Ok(())
    }

    /// Get Discord Gateway URL
    async fn get_gateway_url(&self) -> Result<String> {
        let response = self.client
            .get("https://discord.com/api/v10/gateway/bot")
            .header("Authorization", format!("Bot {}", self.config.bot_token))
            .send()
            .await
            .context("Failed to get Discord gateway URL")?;

        let gateway_info: GatewayInfo = response.json().await
            .context("Failed to parse gateway response")?;

        Ok(gateway_info.url)
    }

    /// Send a message to Discord channel
    async fn post_message(&self, channel_id: &str, content: &str) -> Result<()> {
        let body = serde_json::json!({
            "content": content,
        });

        let response = self.client
            .post(format!("https://discord.com/api/v10/channels/{}/messages", channel_id))
            .header("Authorization", format!("Bot {}", self.config.bot_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Failed to send Discord message")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!("Discord API error: {}", error_text);
        }

        Ok(())
    }

    /// Send a reply to a specific message
    pub async fn reply_to_message(&self, channel_id: &str, message_id: &str, content: &str) -> Result<()> {
        let body = serde_json::json!({
            "content": content,
            "message_reference": {
                "message_id": message_id,
            },
        });

        let response = self.client
            .post(format!("https://discord.com/api/v10/channels/{}/messages", channel_id))
            .header("Authorization", format!("Bot {}", self.config.bot_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Failed to send Discord reply")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!("Discord API error: {}", error_text);
        }

        Ok(())
    }

    /// Add a reaction to a message
    pub async fn add_reaction(&self, channel_id: &str, message_id: &str, emoji: &str) -> Result<()> {
        let emoji_encoded = urlencoding::encode(emoji);

        let response = self.client
            .put(format!(
                "https://discord.com/api/v10/channels/{}/messages/{}/reactions/{}/@me",
                channel_id, message_id, emoji_encoded
            ))
            .header("Authorization", format!("Bot {}", self.config.bot_token))
            .send()
            .await
            .context("Failed to add Discord reaction")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!("Discord API error: {}", error_text);
        }

        Ok(())
    }

    /// Handle incoming Discord event
    async fn handle_gateway_event(&self, event: DiscordGatewayEvent) -> Result<()> {
        match event {
            DiscordGatewayEvent::MessageCreate { channel_id, author, content, id, .. } => {
                // Skip bot's own messages
                if author.bot.unwrap_or(false) {
                    return Ok(());
                }

                // Check if bot is mentioned or command prefix is used
                let should_respond = self.config.respond_to_all ||
                    content.contains(&format!("<@{}>", self.config.application_id)) ||
                    self.config.command_prefix.as_ref().map_or(false, |prefix| content.starts_with(prefix));

                if should_respond {
                    let msg = MessengerMessage {
                        platform: "discord".to_string(),
                        channel_id,
                        user_id: author.id,
                        user_name: author.username,
                        text: content,
                        timestamp: chrono::Utc::now().timestamp(),
                        thread_id: Some(id),
                    };

                    self.event_tx.send(MessengerEvent::Message(msg))
                        .context("Failed to send messenger event")?;
                }
            }
            _ => {
                // Handle other event types as needed
            }
        }

        Ok(())
    }
}

#[async_trait]
impl Messenger for DiscordMessenger {
    async fn start(&self) -> Result<()> {
        self.connect_gateway().await
    }

    async fn stop(&self) -> Result<()> {
        *self.running.write().await = false;
        Ok(())
    }

    async fn send_message(&self, channel_id: &str, message: &str) -> Result<()> {
        self.post_message(channel_id, message).await
    }

    fn platform_name(&self) -> &str {
        "discord"
    }
}

// Discord API types

#[derive(Debug, Deserialize)]
struct GatewayInfo {
    url: String,
}

#[derive(Debug, Deserialize)]
struct DiscordUser {
    id: String,
    username: String,
    bot: Option<bool>,
}

#[derive(Debug)]
enum DiscordGatewayEvent {
    MessageCreate {
        id: String,
        channel_id: String,
        author: DiscordUser,
        content: String,
    },
    MessageReactionAdd {
        message_id: String,
        channel_id: String,
        user_id: String,
        emoji: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discord_config_default() {
        let config = DiscordConfig::default();
        assert_eq!(config.command_prefix, Some("!".to_string()));
        assert!(!config.respond_to_all);
    }

    #[tokio::test]
    async fn test_discord_messenger_creation() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let config = DiscordConfig::default();
        let messenger = DiscordMessenger::new(config, tx);
        assert_eq!(messenger.platform_name(), "discord");
    }
}
