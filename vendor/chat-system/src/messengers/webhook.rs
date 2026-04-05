//! Generic webhook messenger — HTTP POST implementation.

use crate::{Message, Messenger};
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;

pub struct WebhookMessenger {
    name: String,
    url: String,
    client: Client,
    connected: bool,
}

impl WebhookMessenger {
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            client: Client::new(),
            connected: false,
        }
    }
}

#[async_trait]
impl Messenger for WebhookMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "webhook"
    }

    async fn initialize(&mut self) -> Result<()> {
        self.connected = true;
        Ok(())
    }

    async fn send_message(&self, recipient: &str, content: &str) -> Result<String> {
        let body = json!({
            "recipient": recipient,
            "content": content,
            "timestamp": chrono::Utc::now().timestamp(),
        });

        let resp = self.client.post(&self.url).json(&body).send().await?;

        if resp.status().is_success() {
            Ok(format!("webhook:{}", chrono::Utc::now().timestamp_millis()))
        } else {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Webhook POST failed {}: {}", status, text);
        }
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
