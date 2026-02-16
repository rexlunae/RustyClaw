//! Discord messenger using bot token and REST API.

use super::{Message, Messenger, SendOptions};
use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Discord API base URL
const DISCORD_API_BASE: &str = "https://discord.com/api/v10";

/// Discord message from API
#[derive(Debug, Clone, Deserialize)]
struct DiscordMessage {
    id: String,
    channel_id: String,
    author: DiscordUser,
    content: String,
    timestamp: String,
    #[serde(default)]
    referenced_message: Option<Box<DiscordMessage>>,
}

/// Discord user
#[derive(Debug, Clone, Deserialize)]
struct DiscordUser {
    id: String,
    username: String,
    #[serde(default)]
    bot: bool,
}

/// Discord current user (bot)
#[derive(Debug, Deserialize)]
struct DiscordCurrentUser {
    id: String,
}

/// Discord messenger using bot token
pub struct DiscordMessenger {
    name: String,
    bot_token: String,
    connected: bool,
    http: reqwest::Client,
    bot_id: Arc<RwLock<Option<String>>>,
    processed_ids: Arc<RwLock<HashSet<String>>>,
}

impl DiscordMessenger {
    pub fn new(name: String, bot_token: String) -> Self {
        Self {
            name,
            bot_token,
            connected: false,
            http: reqwest::Client::new(),
            bot_id: Arc::new(RwLock::new(None)),
            processed_ids: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    /// Get all guilds (servers) the bot is in
    async fn get_guilds(&self) -> Result<Vec<String>> {
        let url = format!("{}/users/@me/guilds", DISCORD_API_BASE);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("Failed to get guilds: {}", resp.status());
        }

        let guilds: Vec<serde_json::Value> = resp.json().await?;
        Ok(guilds
            .iter()
            .filter_map(|g| g["id"].as_str().map(String::from))
            .collect())
    }

    /// Get text channels for a guild
    async fn get_guild_channels(&self, guild_id: &str) -> Result<Vec<String>> {
        let url = format!("{}/guilds/{}/channels", DISCORD_API_BASE, guild_id);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(Vec::new());
        }

        let channels: Vec<serde_json::Value> = resp.json().await?;
        Ok(channels
            .iter()
            .filter(|c| c["type"].as_u64() == Some(0)) // 0 = GUILD_TEXT
            .filter_map(|c| c["id"].as_str().map(String::from))
            .collect())
    }

    /// Get recent messages from a channel
    async fn get_channel_messages(&self, channel_id: &str, limit: u8) -> Result<Vec<DiscordMessage>> {
        let url = format!(
            "{}/channels/{}/messages?limit={}",
            DISCORD_API_BASE, channel_id, limit
        );
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(Vec::new());
        }

        let messages: Vec<DiscordMessage> = resp.json().await?;
        Ok(messages)
    }
}

#[async_trait]
impl Messenger for DiscordMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "discord"
    }

    async fn initialize(&mut self) -> Result<()> {
        // Verify bot token by fetching current user
        let resp = self
            .http
            .get(&format!("{}/users/@me", DISCORD_API_BASE))
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await?;

        if resp.status().is_success() {
            let user: DiscordCurrentUser = resp.json().await?;
            *self.bot_id.write().await = Some(user.id);
            self.connected = true;
            Ok(())
        } else {
            anyhow::bail!("Discord auth failed: {}", resp.status())
        }
    }

    async fn send_message(&self, channel_id: &str, content: &str) -> Result<String> {
        let url = format!("{}/channels/{}/messages", DISCORD_API_BASE, channel_id);

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bot {}", self.bot_token))
            .json(&serde_json::json!({ "content": content }))
            .send()
            .await?;

        if resp.status().is_success() {
            let data: serde_json::Value = resp.json().await?;
            Ok(data["id"].as_str().unwrap_or("unknown").to_string())
        } else {
            anyhow::bail!("Discord send failed: {}", resp.status())
        }
    }

    async fn send_message_with_options(&self, opts: SendOptions<'_>) -> Result<String> {
        let url = format!("{}/channels/{}/messages", DISCORD_API_BASE, opts.recipient);

        let mut payload = serde_json::json!({ "content": opts.content });

        // Add reply reference if provided
        if let Some(reply_id) = opts.reply_to {
            payload["message_reference"] = serde_json::json!({
                "message_id": reply_id
            });
        }

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bot {}", self.bot_token))
            .json(&payload)
            .send()
            .await?;

        if resp.status().is_success() {
            let data: serde_json::Value = resp.json().await?;
            Ok(data["id"].as_str().unwrap_or("unknown").to_string())
        } else {
            let status = resp.status();
            let error_text = resp.text().await.unwrap_or_else(|_| "unknown".to_string());
            anyhow::bail!("Discord send failed: {} - {}", status, error_text)
        }
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        if !self.is_connected() {
            return Ok(Vec::new());
        }

        let bot_id = self.bot_id.read().await.clone();
        let bot_id = match bot_id {
            Some(id) => id,
            None => return Ok(Vec::new()),
        };

        let mut new_messages = Vec::new();

        // Get all guilds the bot is in
        let guilds = match self.get_guilds().await {
            Ok(g) => g,
            Err(e) => {
                eprintln!("[discord] Failed to get guilds: {}", e);
                return Ok(Vec::new());
            }
        };

        // For each guild, get channels and poll recent messages
        for guild_id in guilds.iter().take(5) {
            // Limit to 5 guilds to avoid rate limits
            let channels = match self.get_guild_channels(guild_id).await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("[discord] Failed to get channels for guild {}: {}", guild_id, e);
                    continue;
                }
            };

            // Poll recent messages from each channel (limit to first 3 channels per guild)
            for channel_id in channels.iter().take(3) {
                let messages = match self.get_channel_messages(channel_id, 10).await {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!(
                            "[discord] Failed to get messages from channel {}: {}",
                            channel_id, e
                        );
                        continue;
                    }
                };

                // Convert Discord messages to our format
                for msg in messages {
                    // Skip bot's own messages
                    if msg.author.id == bot_id {
                        continue;
                    }

                    // Skip bot messages
                    if msg.author.bot {
                        continue;
                    }

                    // Check if already processed
                    {
                        let mut processed = self.processed_ids.write().await;
                        if processed.contains(&msg.id) {
                            continue;
                        }
                        processed.insert(msg.id.clone());

                        // Keep only last 1000 IDs
                        if processed.len() > 1000 {
                            let to_remove: Vec<_> = processed.iter().take(100).cloned().collect();
                            for id in to_remove {
                                processed.remove(&id);
                            }
                        }
                    }

                    // Parse timestamp
                    let timestamp = chrono::DateTime::parse_from_rfc3339(&msg.timestamp)
                        .map(|dt| dt.timestamp())
                        .unwrap_or_else(|_| chrono::Utc::now().timestamp());

                    // Build reply_to from referenced_message
                    let reply_to = msg.referenced_message.as_ref().map(|r| r.id.clone());

                    new_messages.push(Message {
                        id: msg.id,
                        sender: format!("{}#{}", msg.author.username, msg.author.id),
                        content: msg.content,
                        timestamp,
                        channel: Some(msg.channel_id),
                        reply_to,
                        media: None,
                    });
                }
            }
        }

        // Return newest first (reverse chronological)
        new_messages.reverse();
        Ok(new_messages)
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.connected = false;
        Ok(())
    }

    async fn set_typing(&self, channel_id: &str, typing: bool) -> Result<()> {
        if !typing {
            // Discord's typing indicator auto-expires after 10 seconds
            return Ok(());
        }

        let url = format!("{}/channels/{}/typing", DISCORD_API_BASE, channel_id);

        let _ = self
            .http
            .post(&url)
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await;

        // Ignore errors - typing indicator is best-effort
        Ok(())
    }

    async fn set_presence(&self, status: &str) -> Result<()> {
        // Discord requires WebSocket Gateway for presence updates
        // This is a no-op for REST API only implementation
        let _ = status;
        Ok(())
    }
}
