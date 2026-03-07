//! iMessage messenger via BlueBubbles server API.
//!
//! BlueBubbles is a self-hosted iMessage bridge that exposes a REST API.
//! This messenger connects to a BlueBubbles server to send and receive
//! iMessage conversations.
//!
//! Requirements:
//! - A running BlueBubbles server (macOS with iMessage)
//! - Server URL and password configured

use super::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;

/// iMessage messenger via BlueBubbles REST API.
pub struct IMessageMessenger {
    name: String,
    /// BlueBubbles server URL (e.g. "http://localhost:1234").
    server_url: String,
    /// BlueBubbles server password.
    password: String,
    connected: bool,
    http: reqwest::Client,
    /// Last message timestamp for polling.
    last_poll_ts: i64,
}

impl IMessageMessenger {
    pub fn new(name: String, server_url: String, password: String) -> Self {
        Self {
            name,
            server_url: server_url.trim_end_matches('/').to_string(),
            password,
            connected: false,
            http: reqwest::Client::new(),
            last_poll_ts: chrono::Utc::now().timestamp_millis(),
        }
    }

    /// Build a BlueBubbles API URL with password query param.
    fn api_url(&self, path: &str) -> String {
        format!(
            "{}/api/v1/{}?password={}",
            self.server_url, path, self.password
        )
    }
}

#[async_trait]
impl Messenger for IMessageMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "imessage"
    }

    async fn initialize(&mut self) -> Result<()> {
        // Check server connectivity
        let url = self.api_url("server/info");
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .context("Failed to connect to BlueBubbles server")?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "BlueBubbles server returned {} — check URL and password",
                resp.status()
            );
        }

        let data: serde_json::Value = resp.json().await?;
        if data["status"].as_i64() != Some(200) {
            anyhow::bail!(
                "BlueBubbles server error: {}",
                data["message"].as_str().unwrap_or("unknown")
            );
        }

        self.connected = true;
        tracing::info!(
            server = %self.server_url,
            os_version = ?data["data"]["os_version"].as_str(),
            "BlueBubbles/iMessage connected"
        );

        Ok(())
    }

    async fn send_message(&self, chat_guid: &str, content: &str) -> Result<String> {
        let url = self.api_url("message/text");
        let payload = serde_json::json!({
            "chatGuid": chat_guid,
            "message": content,
            "method": "apple-script"
        });

        let resp = self
            .http
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("BlueBubbles send failed")?;

        if resp.status().is_success() {
            let data: serde_json::Value = resp.json().await?;
            return Ok(
                data["data"]["guid"]
                    .as_str()
                    .unwrap_or("sent")
                    .to_string(),
            );
        }

        anyhow::bail!("BlueBubbles send returned {}", resp.status())
    }

    async fn send_message_with_options(&self, opts: SendOptions<'_>) -> Result<String> {
        self.send_message(opts.recipient, opts.content).await
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        let url = format!(
            "{}/api/v1/message?password={}&after={}&limit=50&sort=asc",
            self.server_url, self.password, self.last_poll_ts
        );

        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Ok(Vec::new());
        }

        let data: serde_json::Value = resp.json().await?;
        let messages = data["data"].as_array().cloned().unwrap_or_default();

        let mut result = Vec::new();
        for msg in &messages {
            // Skip messages we sent
            if msg["isFromMe"].as_bool() == Some(true) {
                continue;
            }

            let text = match msg["text"].as_str() {
                Some(t) if !t.is_empty() => t,
                _ => continue,
            };

            let guid = msg["guid"].as_str().unwrap_or("").to_string();
            let sender = msg["handle"]["address"]
                .as_str()
                .or(msg["handle"]["id"].as_str())
                .unwrap_or("unknown")
                .to_string();
            let date_created = msg["dateCreated"].as_i64().unwrap_or(0);
            let chat_guid = msg["chats"]
                .as_array()
                .and_then(|c| c.first())
                .and_then(|c| c["guid"].as_str())
                .map(|s| s.to_string());

            result.push(Message {
                id: guid,
                sender,
                content: text.to_string(),
                timestamp: date_created / 1000, // ms to seconds
                channel: chat_guid,
                reply_to: None,
                media: None,
            });
        }

        Ok(result)
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
    fn test_imessage_creation() {
        let m = IMessageMessenger::new(
            "test".to_string(),
            "http://localhost:1234".to_string(),
            "password123".to_string(),
        );
        assert_eq!(m.name(), "test");
        assert_eq!(m.messenger_type(), "imessage");
        assert!(!m.is_connected());
        assert_eq!(m.server_url, "http://localhost:1234");
    }

    #[test]
    fn test_api_url() {
        let m = IMessageMessenger::new(
            "test".to_string(),
            "http://localhost:1234/".to_string(), // trailing slash
            "pass".to_string(),
        );
        let url = m.api_url("server/info");
        assert!(url.starts_with("http://localhost:1234/api/v1/server/info"));
        assert!(url.contains("password=pass"));
    }
}
