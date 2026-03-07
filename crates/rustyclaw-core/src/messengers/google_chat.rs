//! Google Chat messenger using webhook URLs.
//!
//! Uses Google Chat incoming webhooks for sending messages.
//! For receiving messages, requires a Google Cloud Pub/Sub subscription
//! or the Google Chat API with a service account.

use super::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;

/// Google Chat messenger using webhooks and/or Chat API.
pub struct GoogleChatMessenger {
    name: String,
    /// Incoming webhook URL for sending messages.
    webhook_url: Option<String>,
    /// Service account credentials JSON path (for Chat API).
    credentials_path: Option<String>,
    /// Space name(s) to listen on (e.g. "spaces/AAAA").
    spaces: Vec<String>,
    connected: bool,
    http: reqwest::Client,
}

impl GoogleChatMessenger {
    pub fn new(name: String) -> Self {
        Self {
            name,
            webhook_url: None,
            credentials_path: None,
            spaces: Vec::new(),
            connected: false,
            http: reqwest::Client::new(),
        }
    }

    /// Set the incoming webhook URL.
    pub fn with_webhook_url(mut self, url: String) -> Self {
        self.webhook_url = Some(url);
        self
    }

    /// Set the service account credentials path.
    pub fn with_credentials(mut self, path: String) -> Self {
        self.credentials_path = Some(path);
        self
    }

    /// Set spaces to listen on.
    pub fn with_spaces(mut self, spaces: Vec<String>) -> Self {
        self.spaces = spaces;
        self
    }
}

#[async_trait]
impl Messenger for GoogleChatMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "google_chat"
    }

    async fn initialize(&mut self) -> Result<()> {
        // Validate that we have at least one way to send messages
        if self.webhook_url.is_none() && self.credentials_path.is_none() {
            anyhow::bail!(
                "Google Chat requires either 'webhook_url' or 'credentials_path' \
                 (service account JSON)"
            );
        }

        self.connected = true;
        tracing::info!(
            has_webhook = self.webhook_url.is_some(),
            has_credentials = self.credentials_path.is_some(),
            spaces = ?self.spaces,
            "Google Chat initialized"
        );

        Ok(())
    }

    async fn send_message(&self, space: &str, content: &str) -> Result<String> {
        // Prefer webhook for simplicity
        if let Some(ref webhook_url) = self.webhook_url {
            let payload = serde_json::json!({
                "text": content
            });

            let resp = self
                .http
                .post(webhook_url)
                .json(&payload)
                .send()
                .await
                .context("Google Chat webhook POST failed")?;

            if resp.status().is_success() {
                let data: serde_json::Value = resp.json().await?;
                return Ok(data["name"].as_str().unwrap_or("sent").to_string());
            }
            anyhow::bail!("Google Chat webhook returned {}", resp.status());
        }

        // Fall back to Chat API with service account
        if self.credentials_path.is_some() {
            // Chat API requires OAuth2 — for now, return an error directing
            // the user to use webhook mode or the claw-me-maybe skill.
            anyhow::bail!(
                "Google Chat API (service account) send not yet implemented for space '{}'. \
                 Use webhook_url or the claw-me-maybe skill.",
                space
            );
        }

        anyhow::bail!("No Google Chat send method configured")
    }

    async fn send_message_with_options(&self, opts: SendOptions<'_>) -> Result<String> {
        // Google Chat webhooks don't support threading via webhook URL,
        // so we just send the message.
        self.send_message(opts.recipient, opts.content).await
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        // Google Chat requires either:
        // 1. A Pub/Sub subscription (push or pull)
        // 2. The Chat API with events
        // For now, return empty — receiving requires more complex setup.
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
    fn test_google_chat_creation() {
        let m = GoogleChatMessenger::new("test".to_string())
            .with_webhook_url("https://chat.googleapis.com/v1/spaces/XXX/messages?key=YYY".to_string());
        assert_eq!(m.name(), "test");
        assert_eq!(m.messenger_type(), "google_chat");
        assert!(!m.is_connected());
        assert!(m.webhook_url.is_some());
    }

    #[test]
    fn test_with_credentials() {
        let m = GoogleChatMessenger::new("test".to_string())
            .with_credentials("/path/to/sa.json".to_string())
            .with_spaces(vec!["spaces/AAAA".to_string()]);
        assert_eq!(m.credentials_path, Some("/path/to/sa.json".to_string()));
        assert_eq!(m.spaces, vec!["spaces/AAAA"]);
    }
}
