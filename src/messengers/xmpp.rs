//! XMPP messenger adapter.
//!
//! This implementation targets XMPP HTTP bridge endpoints:
//! - `webhook_url` mode (no auth)
//! - `api_url` + bearer token mode
//!
//! Inbound polling is not implemented and returns an empty list.

use crate::messengers::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct XmppConfig {
    pub webhook_url: Option<String>,
    pub api_url: Option<String>,
    pub token: Option<String>,
    pub from: Option<String>,
    pub default_recipient: Option<String>,
}

impl Default for XmppConfig {
    fn default() -> Self {
        Self {
            webhook_url: None,
            api_url: None,
            token: None,
            from: None,
            default_recipient: None,
        }
    }
}

pub struct XmppMessenger {
    name: String,
    config: XmppConfig,
    http: reqwest::Client,
    connected: bool,
}

impl XmppMessenger {
    pub fn new(name: String, config: XmppConfig) -> Self {
        Self {
            name,
            config,
            http: reqwest::Client::new(),
            connected: false,
        }
    }

    fn recipient_for_send(&self, recipient: &str) -> Result<String> {
        if !recipient.trim().is_empty() {
            return Ok(recipient.to_string());
        }

        self.config
            .default_recipient
            .clone()
            .context("XMPP requires recipient or default_recipient")
    }

    fn api_endpoint(&self) -> Result<String> {
        let base = self
            .config
            .api_url
            .as_ref()
            .context("XMPP API mode requires api_url")?
            .trim_end_matches('/')
            .to_string();

        if base.ends_with("/messages") {
            Ok(base)
        } else {
            Ok(format!("{}/messages", base))
        }
    }
}

#[async_trait]
impl Messenger for XmppMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "xmpp"
    }

    async fn initialize(&mut self) -> Result<()> {
        if self.config.webhook_url.is_some() {
            self.connected = true;
            return Ok(());
        }

        // API mode is considered configured if endpoint and token exist.
        self.api_endpoint()?;
        self.config
            .token
            .as_ref()
            .context("XMPP API mode requires token")?;

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
        let to = self.recipient_for_send(opts.recipient)?;

        let payload = serde_json::json!({
            "to": to,
            "from": self.config.from,
            "body": opts.content,
            "reply_to": opts.reply_to,
        });

        if let Some(webhook_url) = &self.config.webhook_url {
            let resp = self
                .http
                .post(webhook_url)
                .json(&payload)
                .send()
                .await
                .context("Failed to send XMPP webhook message")?;

            if !resp.status().is_success() {
                anyhow::bail!("XMPP webhook send failed: {}", resp.status());
            }

            return Ok(format!("xmpp-{}", chrono::Utc::now().timestamp_millis()));
        }

        let api_url = self.api_endpoint()?;
        let token = self
            .config
            .token
            .as_ref()
            .context("XMPP token is not configured")?;

        let resp = self
            .http
            .post(api_url)
            .bearer_auth(token)
            .json(&payload)
            .send()
            .await
            .context("Failed to send XMPP API message")?;

        if !resp.status().is_success() {
            anyhow::bail!("XMPP API send failed: {}", resp.status());
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse XMPP send response")?;

        Ok(data
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("xmpp-message")
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
    fn test_xmpp_type() {
        let m = XmppMessenger::new("xmpp-main".to_string(), XmppConfig::default());
        assert_eq!(m.messenger_type(), "xmpp");
    }
}
