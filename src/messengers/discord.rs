//! Discord messenger using bot token and REST API.

use super::{Message, Messenger};
use anyhow::Result;
use async_trait::async_trait;

/// Discord messenger using bot token
pub struct DiscordMessenger {
    name: String,
    bot_token: String,
    connected: bool,
    http: reqwest::Client,
}

impl DiscordMessenger {
    pub fn new(name: String, bot_token: String) -> Self {
        Self {
            name,
            bot_token,
            connected: false,
            http: reqwest::Client::new(),
        }
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
            .get("https://discord.com/api/v10/users/@me")
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await?;

        if resp.status().is_success() {
            self.connected = true;
            Ok(())
        } else {
            anyhow::bail!("Discord auth failed: {}", resp.status())
        }
    }

    async fn send_message(&self, channel_id: &str, content: &str) -> Result<String> {
        let url = format!(
            "https://discord.com/api/v10/channels/{}/messages",
            channel_id
        );

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

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        // Real implementation would use Discord gateway WebSocket
        Ok(Vec::new())
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

        let url = format!(
            "https://discord.com/api/v10/channels/{}/typing",
            channel_id
        );

        let _ = self
            .http
            .post(&url)
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await;

        // Ignore errors - typing indicator is best-effort
        Ok(())
    }
}
