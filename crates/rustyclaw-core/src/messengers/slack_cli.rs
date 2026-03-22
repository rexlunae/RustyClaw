//! Slack messenger using Web API.
//!
//! Uses Slack's Web API at https://slack.com/api/
//! Simple REST-based implementation for sending and receiving messages.
//!
//! This requires the `slack-cli` feature to be enabled.

use super::{Message, Messenger};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::Mutex;

const SLACK_API_BASE: &str = "https://slack.com/api";

/// Slack API response wrapper
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SlackResponse<T> {
    ok: bool,
    #[serde(flatten)]
    data: Option<T>,
    error: Option<String>,
}

/// Slack auth.test response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AuthTestResponse {
    user_id: String,
    user: String,
    team_id: String,
    team: String,
}

/// Slack message object
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SlackMessage {
    ts: String,
    user: Option<String>,
    text: String,
    channel: Option<String>,
}

/// Slack conversations.history response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct HistoryResponse {
    messages: Vec<SlackMessage>,
    has_more: bool,
}

/// Slack chat.postMessage response  
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct PostMessageResponse {
    ts: String,
    channel: String,
}

/// Slack messenger implementation
pub struct SlackCliMessenger {
    name: String,
    token: String,
    client: Client,
    connected: Arc<Mutex<bool>>,
    /// Map of channel ID -> last seen message timestamp
    last_timestamps: Arc<Mutex<std::collections::HashMap<String, String>>>,
    /// Channels to watch for incoming messages
    watch_channels: Vec<String>,
}

impl SlackCliMessenger {
    /// Create a new Slack messenger with bot token
    pub fn new(name: String, token: String) -> Self {
        Self {
            name,
            token,
            client: Client::new(),
            connected: Arc::new(Mutex::new(false)),
            last_timestamps: Arc::new(Mutex::new(std::collections::HashMap::new())),
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
        format!("Bearer {}", self.token)
    }

    /// Test authentication
    async fn auth_test(&self) -> Result<AuthTestResponse> {
        let url = format!("{}/auth.test", SLACK_API_BASE);
        let response = self.client
            .post(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await?;
        
        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Slack API error: {} - {}", status, error_text);
        }

        let data: SlackResponse<AuthTestResponse> = response.json().await?;
        if !data.ok {
            anyhow::bail!("Slack auth failed: {}", data.error.unwrap_or_default());
        }

        data.data.ok_or_else(|| anyhow::anyhow!("No auth data in response"))
    }

    /// Send a message to a channel
    async fn post_message(&self, channel: &str, text: &str) -> Result<String> {
        let url = format!("{}/chat.postMessage", SLACK_API_BASE);
        
        let body = serde_json::json!({
            "channel": channel,
            "text": text
        });

        let response = self.client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Failed to send Slack message")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to send message: {} - {}", status, error_text);
        }

        let data: SlackResponse<PostMessageResponse> = response.json().await?;
        if !data.ok {
            anyhow::bail!("Slack post failed: {}", data.error.unwrap_or_default());
        }

        Ok(data.data.map(|d| d.ts).unwrap_or_default())
    }

    /// Get conversation history
    async fn get_history(&self, channel: &str, limit: u32) -> Result<Vec<SlackMessage>> {
        let last_ts = self.last_timestamps.lock().await;
        let oldest = last_ts.get(channel).cloned();
        drop(last_ts);

        let mut url = format!("{}/conversations.history?channel={}&limit={}", 
                             SLACK_API_BASE, channel, limit);
        
        if let Some(ts) = oldest {
            url.push_str(&format!("&oldest={}", ts));
        }

        let response = self.client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await?;
        
        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to get history: {} - {}", status, error_text);
        }

        let data: SlackResponse<HistoryResponse> = response.json().await?;
        if !data.ok {
            anyhow::bail!("Slack history failed: {}", data.error.unwrap_or_default());
        }

        let messages = data.data.map(|h| h.messages).unwrap_or_default();

        // Update last seen timestamp
        if let Some(latest) = messages.first() {
            self.last_timestamps.lock().await.insert(channel.to_string(), latest.ts.clone());
        }

        Ok(messages)
    }
}

impl std::fmt::Debug for SlackCliMessenger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlackCliMessenger")
            .field("name", &self.name)
            .field("watch_channels", &self.watch_channels)
            .finish()
    }
}

#[async_trait]
impl Messenger for SlackCliMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "slack"
    }

    async fn initialize(&mut self) -> Result<()> {
        let auth = self.auth_test().await.context("Failed to verify Slack token")?;
        *self.connected.lock().await = true;
        tracing::info!("Slack connected as {} in team {}", auth.user, auth.team);
        Ok(())
    }

    async fn send_message(&self, recipient: &str, content: &str) -> Result<String> {
        self.post_message(recipient, content).await
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        let mut all_messages = Vec::new();

        for channel in &self.watch_channels.clone() {
            match self.get_history(channel, 100).await {
                Ok(messages) => {
                    for msg in messages {
                        all_messages.push(Message {
                            id: msg.ts.clone(),
                            sender: msg.user.unwrap_or_else(|| "unknown".to_string()),
                            content: msg.text,
                            timestamp: parse_slack_ts(&msg.ts),
                            channel: Some(channel.clone()),
                            reply_to: None,
                            media: None,
                            is_direct: false, // Slack DM detection would need channel type check
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to get history for {}: {}", channel, e);
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

    async fn set_typing(&self, _channel: &str, _typing: bool) -> Result<()> {
        // Slack doesn't have a typing indicator API for bots
        // The users.setPresence endpoint only sets away/auto status
        // Just no-op for now
        Ok(())
    }
}

/// Parse Slack timestamp (e.g., "1234567890.123456") to Unix timestamp
fn parse_slack_ts(ts: &str) -> i64 {
    ts.split('.').next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}
