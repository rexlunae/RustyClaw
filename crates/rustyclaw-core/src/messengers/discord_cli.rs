//! Discord messenger using REST API.
//!
//! Uses Discord's REST API at https://discord.com/api/v10/
//! For simplicity, this uses polling for receiving messages rather than
//! WebSocket Gateway. Good for low-volume bot use cases.
//!
//! This requires the `discord-cli` feature to be enabled.

use super::{Message, Messenger};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::Mutex;

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";

/// Discord User object
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DiscordUser {
    id: String,
    username: String,
    discriminator: String,
}

/// Discord Message object
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DiscordMessage {
    id: String,
    channel_id: String,
    author: DiscordUser,
    content: String,
    timestamp: String,
}

/// Discord messenger implementation
pub struct DiscordCliMessenger {
    name: String,
    token: String,
    client: Client,
    connected: Arc<Mutex<bool>>,
    last_message_ids: Arc<Mutex<std::collections::HashMap<String, String>>>,
    watch_channels: Vec<String>,
}

impl DiscordCliMessenger {
    /// Create a new Discord messenger with bot token
    pub fn new(name: String, token: String) -> Self {
        Self {
            name,
            token,
            client: Client::new(),
            connected: Arc::new(Mutex::new(false)),
            last_message_ids: Arc::new(Mutex::new(std::collections::HashMap::new())),
            watch_channels: Vec::new(),
        }
    }

    /// Add a channel to watch for incoming messages
    pub fn watch_channel(mut self, channel_id: String) -> Self {
        self.watch_channels.push(channel_id);
        self
    }

    /// Get authorization header
    fn auth_header(&self) -> String {
        format!("Bot {}", self.token)
    }

    /// Get current user info to verify token
    async fn get_me(&self) -> Result<DiscordUser> {
        let url = format!("{}/users/@me", DISCORD_API_BASE);
        let response = self.client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await?;
        
        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Discord API error: {} - {}", status, error_text);
        }

        response.json().await.context("Failed to parse user info")
    }

    /// Send a message to a channel
    async fn send_message_internal(&self, channel_id: &str, content: &str) -> Result<String> {
        let url = format!("{}/channels/{}/messages", DISCORD_API_BASE, channel_id);
        
        let body = serde_json::json!({
            "content": content
        });

        let response = self.client
            .post(&url)
            .header("Authorization", self.auth_header())
            .json(&body)
            .send()
            .await
            .context("Failed to send Discord message")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to send message: {} - {}", status, error_text);
        }

        let msg: DiscordMessage = response.json().await?;
        Ok(msg.id)
    }

    /// Get messages from a channel
    async fn get_channel_messages(&self, channel_id: &str, limit: u32) -> Result<Vec<DiscordMessage>> {
        let last_ids = self.last_message_ids.lock().await;
        let after_id = last_ids.get(channel_id).cloned();
        drop(last_ids);

        let mut url = format!("{}/channels/{}/messages?limit={}", DISCORD_API_BASE, channel_id, limit);
        
        if let Some(after) = after_id {
            url.push_str(&format!("&after={}", after));
        }

        let response = self.client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await?;
        
        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to get messages: {} - {}", status, error_text);
        }

        let messages: Vec<DiscordMessage> = response.json().await?;

        // Update last seen message ID
        if let Some(latest) = messages.first() {
            self.last_message_ids.lock().await.insert(channel_id.to_string(), latest.id.clone());
        }

        Ok(messages)
    }
}

impl std::fmt::Debug for DiscordCliMessenger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscordCliMessenger")
            .field("name", &self.name)
            .field("watch_channels", &self.watch_channels)
            .finish()
    }
}

#[async_trait]
impl Messenger for DiscordCliMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "discord"
    }

    async fn initialize(&mut self) -> Result<()> {
        let user = self.get_me().await.context("Failed to verify Discord bot token")?;
        *self.connected.lock().await = true;
        tracing::info!("Discord bot connected as {}#{}", user.username, user.discriminator);
        Ok(())
    }

    async fn send_message(&self, recipient: &str, content: &str) -> Result<String> {
        self.send_message_internal(recipient, content).await
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        let mut all_messages = Vec::new();

        for channel in &self.watch_channels {
            match self.get_channel_messages(channel, 100).await {
                Ok(messages) => {
                    for msg in messages {
                        all_messages.push(Message {
                            id: msg.id,
                            sender: msg.author.username,
                            content: msg.content,
                            timestamp: 0, // Would need to parse ISO timestamp
                            channel: Some(msg.channel_id),
                            reply_to: None,
                            media: None,
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to get messages for channel {}: {}", channel, e);
                }
            }
        }

        Ok(all_messages)
    }

    fn is_connected(&self) -> bool {
        self.connected.try_lock().map(|g| *g).unwrap_or(false)
    }

    async fn disconnect(&mut self) -> Result<()> {
        *self.connected.lock().await = false;
        Ok(())
    }

    async fn set_typing(&self, channel: &str, typing: bool) -> Result<()> {
        if !typing {
            return Ok(()); // Discord typing auto-expires after ~10 seconds
        }

        let url = format!("{}/channels/{}/typing", DISCORD_API_BASE, channel);
        
        let response = self.client
            .post(&url)
            .header("Authorization", format!("Bot {}", self.token))
            .send()
            .await
            .context("Failed to send typing indicator")?;

        if !response.status().is_success() {
            // Non-fatal, just log
            eprintln!("Failed to send Discord typing indicator: {}", response.status());
        }

        Ok(())
    }
}
