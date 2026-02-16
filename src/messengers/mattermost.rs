//! Mattermost messenger.
//!
//! Supports:
//! - Incoming webhook mode (`webhook_url`)
//! - REST API mode (`token` + `base_url` + channel id)
//!
//! Inbound polling is not implemented and returns an empty list.

use crate::messengers::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct MattermostConfig {
    pub token: Option<String>,
    pub webhook_url: Option<String>,
    pub base_url: String,
    pub default_channel: Option<String>,
}

impl Default for MattermostConfig {
    fn default() -> Self {
        Self {
            token: None,
            webhook_url: None,
            base_url: "http://localhost:8065".to_string(),
            default_channel: None,
        }
    }
}

pub struct MattermostMessenger {
    name: String,
    config: MattermostConfig,
    http: reqwest::Client,
    connected: bool,
}

impl MattermostMessenger {
    pub fn new(name: String, config: MattermostConfig) -> Self {
        Self {
            name,
            config,
            http: reqwest::Client::new(),
            connected: false,
        }
    }

    fn api_base(&self) -> String {
        self.config.base_url.trim_end_matches('/').to_string()
    }

    fn channel_for_send(&self, recipient: &str) -> Result<String> {
        if !recipient.trim().is_empty() {
            return Ok(recipient.to_string());
        }

        self.config
            .default_channel
            .clone()
            .context("Mattermost requires recipient channel id or default_channel")
    }
}

#[async_trait]
impl Messenger for MattermostMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "mattermost"
    }

    async fn initialize(&mut self) -> Result<()> {
        if self.config.webhook_url.is_some() {
            self.connected = true;
            return Ok(());
        }

        let token = self
            .config
            .token
            .as_ref()
            .context("Mattermost API mode requires token")?;
        let url = format!("{}/api/v4/users/me", self.api_base());

        let resp = self
            .http
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .context("Failed to contact Mattermost API")?;

        if !resp.status().is_success() {
            anyhow::bail!("Mattermost auth failed: {}", resp.status());
        }

        self.connected = true;
        Ok(())
    }

    async fn send_message(&self, recipient: &str, content: &str) -> Result<String> {
        self.send_message_with_options(SendOptions {
            recipient,
            content,
            ..Default::default()
        })
        .await
    }

    async fn send_message_with_options(&self, opts: SendOptions<'_>) -> Result<String> {
        if let Some(webhook_url) = &self.config.webhook_url {
            let mut payload = serde_json::json!({ "text": opts.content });
            let target = if opts.recipient.is_empty() {
                self.config.default_channel.as_deref()
            } else {
                Some(opts.recipient)
            };
            if let Some(channel) = target {
                payload["channel"] = serde_json::Value::String(channel.to_string());
            }

            let resp = self
                .http
                .post(webhook_url)
                .json(&payload)
                .send()
                .await
                .context("Failed to send Mattermost webhook message")?;

            if !resp.status().is_success() {
                anyhow::bail!("Mattermost webhook send failed: {}", resp.status());
            }

            return Ok(format!(
                "mattermost-{}",
                chrono::Utc::now().timestamp_millis()
            ));
        }

        let token = self
            .config
            .token
            .as_ref()
            .context("Mattermost token is not configured")?;
        let channel_id = self.channel_for_send(opts.recipient)?;
        let url = format!("{}/api/v4/posts", self.api_base());

        let mut payload = serde_json::json!({
            "channel_id": channel_id,
            "message": opts.content,
        });
        if let Some(reply_to) = opts.reply_to {
            payload["root_id"] = serde_json::Value::String(reply_to.to_string());
        }

        let resp = self
            .http
            .post(url)
            .bearer_auth(token)
            .json(&payload)
            .send()
            .await
            .context("Failed to send Mattermost API message")?;

        if !resp.status().is_success() {
            anyhow::bail!("Mattermost API send failed: {}", resp.status());
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Mattermost send response")?;

        Ok(data
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("mattermost-message")
            .to_string())
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        Ok(Vec::new())
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
    fn test_mattermost_type() {
        let m = MattermostMessenger::new("mm-main".to_string(), MattermostConfig::default());
        assert_eq!(m.messenger_type(), "mattermost");
    }
}
