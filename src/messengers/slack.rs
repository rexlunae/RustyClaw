//! Slack messenger implementation using Slack Web API and RTM
//!
//! This implementation uses Slack's Web API for sending messages and
//! polling for new messages. For real-time updates, Slack recommends
//! using Socket Mode or Events API with webhooks.
//!
//! Setup:
//! 1. Create a Slack App at https://api.slack.com/apps
//! 2. Add Bot Token Scopes: chat:write, channels:history, channels:read, users:read
//! 3. Install app to workspace
//! 4. Copy Bot User OAuth Token
//! 5. Configure in config.toml

use crate::messengers::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Slack API base URL
const SLACK_API_BASE: &str = "https://slack.com/api";

/// Slack messenger configuration
#[derive(Debug, Clone)]
pub struct SlackConfig {
    /// Bot user OAuth token (xoxb-...)
    pub token: String,
    /// Default channel to monitor (e.g., "C1234567890")
    pub channel: Option<String>,
    /// Polling interval in seconds
    pub poll_interval: u64,
}

/// Slack API response wrapper
#[derive(Debug, Deserialize)]
struct SlackResponse<T> {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(flatten)]
    data: Option<T>,
}

/// Conversations history response
#[derive(Debug, Deserialize)]
struct ConversationsHistory {
    messages: Vec<SlackMessage>,
    #[serde(default)]
    has_more: bool,
}

/// Slack message
#[derive(Debug, Clone, Deserialize)]
struct SlackMessage {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    text: Option<String>,
    ts: String,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    thread_ts: Option<String>,
}

/// Post message response
#[derive(Debug, Deserialize)]
struct PostMessageResponse {
    ts: String,
}

/// Slack messenger
pub struct SlackMessenger {
    name: String,
    config: SlackConfig,
    http: Client,
    last_ts: Arc<RwLock<Option<String>>>,
    processed_ts: Arc<RwLock<Vec<String>>>,
}

impl SlackMessenger {
    /// Create a new Slack messenger
    pub fn new(name: String, config: SlackConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self {
            name,
            config,
            http,
            last_ts: Arc::new(RwLock::new(None)),
            processed_ts: Arc::new(RwLock::new(Vec::new())),
        })
    }

    /// Call a Slack API method
    async fn api_call<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        params: &serde_json::Value,
    ) -> Result<T> {
        let url = format!("{}/{}", SLACK_API_BASE, method);

        let resp: SlackResponse<T> = self
            .http
            .post(&url)
            .bearer_auth(&self.config.token)
            .json(params)
            .send()
            .await
            .context("Failed to call Slack API")?
            .json()
            .await
            .context("Failed to parse Slack API response")?;

        if !resp.ok {
            anyhow::bail!(
                "Slack API error: {}",
                resp.error.unwrap_or_else(|| "unknown".to_string())
            );
        }

        resp.data.context("Missing response data")
    }

    /// Get conversation history
    async fn get_history(&self, channel: &str, limit: usize) -> Result<Vec<SlackMessage>> {
        let mut params = serde_json::json!({
            "channel": channel,
            "limit": limit,
        });

        // Only get messages after last seen timestamp
        if let Some(ref ts) = *self.last_ts.read().await {
            params["oldest"] = serde_json::Value::String(ts.clone());
        }

        let history: ConversationsHistory = self
            .api_call("conversations.history", &params)
            .await
            .context("Failed to get conversation history")?;

        Ok(history.messages)
    }

    /// Post a message to a channel
    async fn post_message(&self, channel: &str, text: &str, thread_ts: Option<&str>) -> Result<String> {
        let mut params = serde_json::json!({
            "channel": channel,
            "text": text,
        });

        if let Some(ts) = thread_ts {
            params["thread_ts"] = serde_json::Value::String(ts.to_string());
        }

        let resp: PostMessageResponse = self
            .api_call("chat.postMessage", &params)
            .await
            .context("Failed to post message")?;

        Ok(resp.ts)
    }

    /// Get user info
    async fn get_user_name(&self, user_id: &str) -> Result<String> {
        #[derive(Deserialize)]
        struct UserInfo {
            user: UserProfile,
        }

        #[derive(Deserialize)]
        struct UserProfile {
            #[serde(default)]
            real_name: Option<String>,
            #[serde(default)]
            name: Option<String>,
        }

        let params = serde_json::json!({
            "user": user_id,
        });

        let info: UserInfo = self
            .api_call("users.info", &params)
            .await
            .context("Failed to get user info")?;

        Ok(info
            .user
            .real_name
            .or(info.user.name)
            .unwrap_or_else(|| user_id.to_string()))
    }
}

#[async_trait]
impl Messenger for SlackMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "slack"
    }

    async fn initialize(&mut self) -> Result<()> {
        // Test API connection
        #[derive(Deserialize)]
        struct AuthTest {
            #[serde(default)]
            user: Option<String>,
        }

        let _test: AuthTest = self
            .api_call("auth.test", &serde_json::json!({}))
            .await
            .context("Failed to authenticate with Slack API")?;

        eprintln!("[slack] Initialized successfully");
        Ok(())
    }

    async fn send_message(&self, recipient: &str, content: &str) -> Result<String> {
        self.post_message(recipient, content, None).await
    }

    async fn send_message_with_options(&self, opts: SendOptions<'_>) -> Result<String> {
        self.post_message(opts.recipient, opts.content, opts.reply_to)
            .await
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        // Get channel to monitor
        let channel = match &self.config.channel {
            Some(ch) => ch,
            None => {
                // No channel configured, return empty
                return Ok(Vec::new());
            }
        };

        // Fetch recent messages
        let slack_messages = self.get_history(channel, 10).await?;

        let mut messages = Vec::new();
        let mut processed = self.processed_ts.write().await;

        for msg in slack_messages {
            // Skip if already processed
            if processed.contains(&msg.ts) {
                continue;
            }

            // Only process regular messages with text
            if msg.msg_type != "message" || msg.text.is_none() || msg.user.is_none() {
                continue;
            }

            // Skip bot messages
            let user_id = msg.user.as_ref().unwrap();
            if user_id.starts_with("B") {
                // Bot users start with 'B'
                continue;
            }

            // Get user name (cache this in production)
            let sender = self
                .get_user_name(user_id)
                .await
                .unwrap_or_else(|_| user_id.clone());

            // Parse timestamp
            let timestamp = msg
                .ts
                .split('.')
                .next()
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0);

            let message = Message {
                id: msg.ts.clone(),
                sender,
                content: msg.text.unwrap_or_default(),
                timestamp,
                channel: msg.channel.or_else(|| Some(channel.clone())),
                reply_to: msg.thread_ts,
                media: None,
            };

            messages.push(message);

            // Update last seen timestamp
            *self.last_ts.write().await = Some(msg.ts.clone());

            // Add to processed list (keep last 100)
            processed.push(msg.ts);
            if processed.len() > 100 {
                processed.drain(0..50);
            }
        }

        Ok(messages)
    }

    fn is_connected(&self) -> bool {
        !self.config.token.is_empty()
    }

    async fn disconnect(&mut self) -> Result<()> {
        *self.last_ts.write().await = None;
        Ok(())
    }

    async fn set_typing(&self, channel: &str, typing: bool) -> Result<()> {
        if !typing {
            return Ok(());
        }

        // Slack doesn't have a direct typing indicator API for bots
        // This is a no-op for now
        let _ = channel;
        Ok(())
    }
}
