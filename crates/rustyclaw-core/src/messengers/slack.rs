//! Slack messenger using Bot Token and Web API.
//!
//! Uses the Slack Web API (chat.postMessage, conversations.history) with a bot token.
//! Supports channels, DMs, and threaded replies.

use super::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::sync::Mutex;

/// Slack messenger using bot token and Web API.
pub struct SlackMessenger {
    name: String,
    bot_token: String,
    /// App-level token for Socket Mode (optional, for real-time events).
    app_token: Option<String>,
    /// Default channel to listen on (if not specified per-message).
    default_channel: Option<String>,
    connected: bool,
    http: reqwest::Client,
    /// Bot user ID (resolved on initialize).
    bot_user_id: Option<String>,
    /// Track last read timestamp per channel for polling.
    /// Wrapped in a Mutex so receive_messages(&self) can update it.
    last_ts: Mutex<std::collections::HashMap<String, String>>,
}

impl SlackMessenger {
    pub fn new(name: String, bot_token: String) -> Self {
        Self {
            name,
            bot_token,
            app_token: None,
            default_channel: None,
            connected: false,
            http: reqwest::Client::new(),
            bot_user_id: None,
            last_ts: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Set the app-level token for Socket Mode.
    pub fn with_app_token(mut self, token: String) -> Self {
        self.app_token = Some(token);
        self
    }

    /// Set the default channel to listen on.
    pub fn with_default_channel(mut self, channel: String) -> Self {
        self.default_channel = Some(channel);
        self
    }

    fn api_url(method: &str) -> String {
        format!("https://slack.com/api/{}", method)
    }

    /// Make an authenticated API call.
    async fn api_call(&self, method: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
        let resp = self
            .http
            .post(Self::api_url(method))
            .header("Authorization", format!("Bearer {}", self.bot_token))
            .header("Content-Type", "application/json; charset=utf-8")
            .json(body)
            .send()
            .await
            .with_context(|| format!("Slack API call to {} failed", method))?;

        let status = resp.status();
        let data: serde_json::Value = resp
            .json()
            .await
            .with_context(|| format!("Failed to parse Slack {} response", method))?;

        if !status.is_success() {
            anyhow::bail!("Slack {} returned HTTP {}", method, status);
        }

        if data["ok"].as_bool() != Some(true) {
            let error = data["error"].as_str().unwrap_or("unknown_error");
            anyhow::bail!("Slack {} error: {}", method, error);
        }

        Ok(data)
    }

    /// Fetch conversation history for a channel since last known timestamp.
    async fn fetch_history(&self, channel: &str) -> Result<Vec<Message>> {
        let mut params = serde_json::json!({
            "channel": channel,
            "limit": 20
        });

        {
            let last_ts = self.last_ts.lock().unwrap();
            if let Some(ts) = last_ts.get(channel) {
                params["oldest"] = serde_json::json!(ts);
            }
        }

        let data = self.api_call("conversations.history", &params).await?;

        let messages_arr = data["messages"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        let mut result = Vec::new();

        for msg in &messages_arr {
            // Skip bot's own messages
            if let Some(user) = msg["user"].as_str() {
                if Some(user) == self.bot_user_id.as_deref() {
                    continue;
                }
            }

            // Skip messages without text
            let text = match msg["text"].as_str() {
                Some(t) if !t.is_empty() => t,
                _ => continue,
            };

            let ts = msg["ts"].as_str().unwrap_or("0").to_string();
            let sender = msg["user"].as_str().unwrap_or("unknown").to_string();

            result.push(Message {
                id: ts.clone(),
                sender,
                content: text.to_string(),
                timestamp: parse_slack_ts(&ts),
                channel: Some(channel.to_string()),
                reply_to: msg["thread_ts"].as_str().map(|s| s.to_string()),
                media: None,
                                        is_direct: false, // TODO: implement DM detection
            });
        }

        // Update last seen timestamp
        if let Some(newest) = messages_arr
            .first()
            .and_then(|m| m["ts"].as_str())
        {
            let mut last_ts = self.last_ts.lock().unwrap();
            last_ts.insert(channel.to_string(), newest.to_string());
        }

        // Slack returns newest first, reverse for chronological order
        result.reverse();
        Ok(result)
    }
}

/// Parse a Slack timestamp (e.g. "1234567890.123456") into epoch seconds.
fn parse_slack_ts(ts: &str) -> i64 {
    ts.split('.')
        .next()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
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
        // Verify bot token with auth.test
        let data = self
            .api_call("auth.test", &serde_json::json!({}))
            .await
            .context("Slack auth.test failed — check your bot token")?;

        self.bot_user_id = data["user_id"].as_str().map(|s| s.to_string());
        self.connected = true;

        tracing::info!(
            bot_user = ?self.bot_user_id,
            team = ?data["team"].as_str(),
            "Slack connected"
        );

        Ok(())
    }

    async fn send_message(&self, channel: &str, content: &str) -> Result<String> {
        let data = self
            .api_call(
                "chat.postMessage",
                &serde_json::json!({
                    "channel": channel,
                    "text": content,
                    "unfurl_links": false,
                }),
            )
            .await?;

        Ok(data["ts"].as_str().unwrap_or("unknown").to_string())
    }

    async fn send_message_with_options(&self, opts: SendOptions<'_>) -> Result<String> {
        let mut payload = serde_json::json!({
            "channel": opts.recipient,
            "text": opts.content,
            "unfurl_links": false,
        });

        // Thread reply
        if let Some(thread_ts) = opts.reply_to {
            payload["thread_ts"] = serde_json::json!(thread_ts);
        }

        let data = self.api_call("chat.postMessage", &payload).await?;
        Ok(data["ts"].as_str().unwrap_or("unknown").to_string())
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        let channel = match &self.default_channel {
            Some(ch) => ch.clone(),
            None => return Ok(Vec::new()),
        };
        self.fetch_history(&channel).await
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.connected = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slack_messenger_creation() {
        let messenger = SlackMessenger::new("test".to_string(), "xoxb-fake".to_string());
        assert_eq!(messenger.name(), "test");
        assert_eq!(messenger.messenger_type(), "slack");
        assert!(!messenger.is_connected());
    }

    #[test]
    fn test_parse_slack_ts() {
        assert_eq!(parse_slack_ts("1234567890.123456"), 1234567890);
        assert_eq!(parse_slack_ts("0"), 0);
        assert_eq!(parse_slack_ts(""), 0);
    }

    #[test]
    fn test_with_options() {
        let messenger = SlackMessenger::new("test".to_string(), "xoxb-fake".to_string())
            .with_app_token("xapp-fake".to_string())
            .with_default_channel("C12345".to_string());
        assert_eq!(messenger.app_token, Some("xapp-fake".to_string()));
        assert_eq!(messenger.default_channel, Some("C12345".to_string()));
    }
}
