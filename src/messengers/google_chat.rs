//! Google Chat messenger.
//!
//! Supports:
//! - Incoming webhook mode (`webhook_url`)
//! - REST API mode (`token` + `space`)
//!
//! Inbound reading is not implemented yet and returns an empty list.

use crate::messengers::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;

const CHAT_API_BASE: &str = "https://chat.googleapis.com/v1";

#[derive(Debug, Clone)]
pub struct GoogleChatConfig {
    pub token: Option<String>,
    pub webhook_url: Option<String>,
    pub space: Option<String>,
    pub api_base: String,
}

pub struct GoogleChatMessenger {
    name: String,
    config: GoogleChatConfig,
    http: reqwest::Client,
    connected: bool,
}

impl GoogleChatMessenger {
    pub fn new(name: String, config: GoogleChatConfig) -> Self {
        Self {
            name,
            config,
            http: reqwest::Client::new(),
            connected: false,
        }
    }
}

#[async_trait]
impl Messenger for GoogleChatMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "google-chat"
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
            .context("Google Chat API mode requires token")?;
        let space = self
            .config
            .space
            .as_ref()
            .context("Google Chat API mode requires space (e.g. spaces/AAAA...)")?;

        let url = format!("{}/{}", self.config.api_base, space);
        let resp = self
            .http
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .context("Failed to contact Google Chat API")?;

        if !resp.status().is_success() {
            anyhow::bail!("Google Chat auth failed: {}", resp.status());
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
            let resp = self
                .http
                .post(webhook_url)
                .json(&serde_json::json!({ "text": opts.content }))
                .send()
                .await
                .context("Failed to send Google Chat webhook message")?;

            if !resp.status().is_success() {
                anyhow::bail!("Google Chat webhook send failed: {}", resp.status());
            }

            return Ok(format!(
                "google-chat-{}",
                chrono::Utc::now().timestamp_millis()
            ));
        }

        let token = self
            .config
            .token
            .as_ref()
            .context("Google Chat token is not configured")?;
        let space = if opts.recipient.is_empty() {
            self.config
                .space
                .as_ref()
                .context("Google Chat space is not configured")?
                .clone()
        } else {
            opts.recipient.to_string()
        };

        let url = format!("{}/{}/messages", self.config.api_base, space);
        let resp = self
            .http
            .post(url)
            .bearer_auth(token)
            .json(&serde_json::json!({ "text": opts.content }))
            .send()
            .await
            .context("Failed to send Google Chat API message")?;

        if !resp.status().is_success() {
            anyhow::bail!("Google Chat API send failed: {}", resp.status());
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Google Chat send response")?;

        Ok(data
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("google-chat-message")
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

impl Default for GoogleChatConfig {
    fn default() -> Self {
        Self {
            token: None,
            webhook_url: None,
            space: None,
            api_base: CHAT_API_BASE.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_google_chat_type() {
        let m = GoogleChatMessenger::new("gc".to_string(), GoogleChatConfig::default());
        assert_eq!(m.messenger_type(), "google-chat");
    }
}
