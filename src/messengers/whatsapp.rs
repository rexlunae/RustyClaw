//! WhatsApp Business Cloud API messenger.
//!
//! This integration supports outbound messaging via Meta Graph API.
//! Inbound messaging is webhook-driven in WhatsApp Cloud API, so
//! `receive_messages` currently returns no messages.

use crate::messengers::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;

/// WhatsApp Cloud API base URL.
const GRAPH_BASE: &str = "https://graph.facebook.com";

#[derive(Debug, Clone)]
pub struct WhatsAppConfig {
    pub token: String,
    pub phone_number_id: String,
    pub api_version: String,
}

#[derive(Debug, Deserialize)]
struct SendResponse {
    #[serde(default)]
    messages: Vec<SentMessage>,
}

#[derive(Debug, Deserialize)]
struct SentMessage {
    id: String,
}

pub struct WhatsAppMessenger {
    name: String,
    config: WhatsAppConfig,
    http: reqwest::Client,
    connected: bool,
}

impl WhatsAppMessenger {
    pub fn new(name: String, config: WhatsAppConfig) -> Self {
        Self {
            name,
            config,
            http: reqwest::Client::new(),
            connected: false,
        }
    }

    fn api_url(&self, suffix: &str) -> String {
        format!(
            "{}/{}/{}/{}",
            GRAPH_BASE, self.config.api_version, self.config.phone_number_id, suffix
        )
    }
}

#[async_trait]
impl Messenger for WhatsAppMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "whatsapp"
    }

    async fn initialize(&mut self) -> Result<()> {
        // Lightweight auth check against phone number node.
        let url = format!(
            "{}/{}/{}?fields=id",
            GRAPH_BASE, self.config.api_version, self.config.phone_number_id
        );

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("Failed to contact WhatsApp Cloud API")?;

        if !resp.status().is_success() {
            anyhow::bail!("WhatsApp auth failed: {}", resp.status());
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
        let payload = serde_json::json!({
            "messaging_product": "whatsapp",
            "to": opts.recipient,
            "type": "text",
            "text": {
                "preview_url": false,
                "body": opts.content,
            }
        });

        let resp = self
            .http
            .post(self.api_url("messages"))
            .bearer_auth(&self.config.token)
            .json(&payload)
            .send()
            .await
            .context("Failed to call WhatsApp send API")?;

        if !resp.status().is_success() {
            anyhow::bail!("WhatsApp send failed: {}", resp.status());
        }

        let data: SendResponse = resp
            .json()
            .await
            .context("Failed to parse WhatsApp send response")?;

        Ok(data
            .messages
            .first()
            .map(|m| m.id.clone())
            .unwrap_or_else(|| format!("whatsapp-{}", chrono::Utc::now().timestamp_millis())))
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        // WhatsApp Cloud API delivers inbound messages via webhooks.
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
    fn test_whatsapp_type() {
        let cfg = WhatsAppConfig {
            token: "token".to_string(),
            phone_number_id: "123".to_string(),
            api_version: "v20.0".to_string(),
        };
        let m = WhatsAppMessenger::new("wa".to_string(), cfg);
        assert_eq!(m.messenger_type(), "whatsapp");
    }
}
