//! Microsoft Teams messenger using incoming webhooks.
//!
//! Uses Teams incoming webhook connectors for sending messages.
//! For bidirectional messaging, a Teams Bot Framework registration is required.

use super::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;

/// Microsoft Teams messenger using webhooks / Bot Framework.
pub struct TeamsMessenger {
    name: String,
    /// Incoming webhook URL for sending messages.
    webhook_url: Option<String>,
    /// Bot Framework app ID (for bidirectional messaging).
    app_id: Option<String>,
    /// Bot Framework app password.
    app_password: Option<String>,
    connected: bool,
    http: reqwest::Client,
}

impl TeamsMessenger {
    pub fn new(name: String) -> Self {
        Self {
            name,
            webhook_url: None,
            app_id: None,
            app_password: None,
            connected: false,
            http: reqwest::Client::new(),
        }
    }

    /// Set the incoming webhook URL.
    pub fn with_webhook_url(mut self, url: String) -> Self {
        self.webhook_url = Some(url);
        self
    }

    /// Set Bot Framework credentials.
    pub fn with_bot_framework(mut self, app_id: String, app_password: String) -> Self {
        self.app_id = Some(app_id);
        self.app_password = Some(app_password);
        self
    }
}

#[async_trait]
impl Messenger for TeamsMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "teams"
    }

    async fn initialize(&mut self) -> Result<()> {
        if self.webhook_url.is_none() && self.app_id.is_none() {
            anyhow::bail!(
                "Teams requires either 'webhook_url' (incoming webhook) or \
                 'app_id'+'app_password' (Bot Framework)"
            );
        }
        self.connected = true;
        tracing::info!(
            has_webhook = self.webhook_url.is_some(),
            has_bot_framework = self.app_id.is_some(),
            "Teams initialized"
        );
        Ok(())
    }

    async fn send_message(&self, _channel: &str, content: &str) -> Result<String> {
        if let Some(ref webhook_url) = self.webhook_url {
            // Teams Adaptive Card or simple text
            let payload = serde_json::json!({
                "@type": "MessageCard",
                "@context": "http://schema.org/extensions",
                "text": content
            });

            let resp = self
                .http
                .post(webhook_url)
                .json(&payload)
                .send()
                .await
                .context("Teams webhook POST failed")?;

            if resp.status().is_success() {
                return Ok(format!("teams-{}", chrono::Utc::now().timestamp_millis()));
            }
            anyhow::bail!("Teams webhook returned {}", resp.status());
        }

        // Bot Framework send requires conversation reference — not yet implemented
        anyhow::bail!(
            "Teams Bot Framework send not yet implemented. Use webhook_url or the claw-me-maybe skill."
        )
    }

    async fn send_message_with_options(&self, opts: SendOptions<'_>) -> Result<String> {
        self.send_message(opts.recipient, opts.content).await
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        // Receiving requires Bot Framework with an HTTP endpoint
        // or Graph API subscription. Return empty for now.
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
    fn test_teams_creation() {
        let m = TeamsMessenger::new("test".to_string())
            .with_webhook_url("https://outlook.office.com/webhook/xxx".to_string());
        assert_eq!(m.name(), "test");
        assert_eq!(m.messenger_type(), "teams");
        assert!(!m.is_connected());
    }

    #[test]
    fn test_with_bot_framework() {
        let m = TeamsMessenger::new("test".to_string())
            .with_bot_framework("app-id".to_string(), "app-pass".to_string());
        assert_eq!(m.app_id, Some("app-id".to_string()));
        assert_eq!(m.app_password, Some("app-pass".to_string()));
    }
}
