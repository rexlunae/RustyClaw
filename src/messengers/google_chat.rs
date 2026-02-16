//! Google Chat messenger.
//!
//! Supports:
//! - Incoming webhook mode (`webhook_url`)
//! - REST API mode (`token` + `space`)
//!
//! Message polling uses Google Chat API to fetch recent space messages.

use crate::messengers::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

const CHAT_API_BASE: &str = "https://chat.googleapis.com/v1";

/// Google Chat message from API
#[derive(Debug, Clone, Deserialize)]
struct ChatMessage {
    name: String,
    sender: ChatUser,
    #[serde(default)]
    text: Option<String>,
    #[serde(rename = "createTime")]
    create_time: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ChatUser {
    name: String,
    #[serde(rename = "displayName")]
    display_name: String,
    #[serde(rename = "type")]
    user_type: String,
}

#[derive(Debug, Deserialize)]
struct ListMessagesResponse {
    messages: Option<Vec<ChatMessage>>,
}

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
    processed_ids: Arc<RwLock<HashSet<String>>>,
}

impl GoogleChatMessenger {
    pub fn new(name: String, config: GoogleChatConfig) -> Self {
        Self {
            name,
            config,
            http: reqwest::Client::new(),
            connected: false,
            processed_ids: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    /// Fetch recent messages from a Google Chat space
    async fn fetch_space_messages(&self, space: &str) -> Result<Vec<ChatMessage>> {
        let token = self
            .config
            .token
            .as_ref()
            .context("Google Chat API requires token")?;

        let url = format!("{}/{}/messages?pageSize=20", self.config.api_base, space);

        let resp = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .context("Failed to fetch Google Chat messages")?;

        if !resp.status().is_success() {
            anyhow::bail!("Google Chat fetch messages failed: {}", resp.status());
        }

        let response: ListMessagesResponse = resp
            .json()
            .await
            .context("Failed to parse Google Chat messages response")?;

        Ok(response.messages.unwrap_or_default())
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
        // Webhook mode doesn't support receiving messages
        if self.config.webhook_url.is_some() || self.config.token.is_none() {
            return Ok(Vec::new());
        }

        if !self.is_connected() {
            return Ok(Vec::new());
        }

        // Get space ID
        let space = match &self.config.space {
            Some(s) => s.clone(),
            None => return Ok(Vec::new()),
        };

        // Fetch recent messages
        let chat_messages = match self.fetch_space_messages(&space).await {
            Ok(msgs) => msgs,
            Err(e) => {
                eprintln!("[google-chat] Failed to fetch messages: {}", e);
                return Ok(Vec::new());
            }
        };

        let mut new_messages = Vec::new();

        for msg in chat_messages {
            // Check if already processed
            {
                let mut processed = self.processed_ids.write().await;
                if processed.contains(&msg.name) {
                    continue;
                }
                processed.insert(msg.name.clone());

                // Keep only last 1000 IDs
                if processed.len() > 1000 {
                    let to_remove: Vec<_> = processed.iter().take(100).cloned().collect();
                    for id in to_remove {
                        processed.remove(&id);
                    }
                }
            }

            // Skip bot messages
            if msg.sender.user_type == "BOT" {
                continue;
            }

            // Get message text
            let content = msg.text.unwrap_or_default();

            // Skip empty messages
            if content.trim().is_empty() {
                continue;
            }

            // Parse timestamp
            let timestamp = chrono::DateTime::parse_from_rfc3339(&msg.create_time)
                .map(|dt| dt.timestamp())
                .unwrap_or_else(|_| chrono::Utc::now().timestamp());

            new_messages.push(Message {
                id: msg.name.clone(),
                sender: msg.sender.display_name,
                content,
                timestamp,
                channel: Some(space.clone()),
                reply_to: None,
                media: None,
            });
        }

        // Return newest first (reverse chronological)
        new_messages.reverse();
        Ok(new_messages)
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
