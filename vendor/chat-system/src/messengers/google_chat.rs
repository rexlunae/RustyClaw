//! Google Chat messenger — Incoming Webhook and Google Chat API implementation.

use crate::{Message, Messenger};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::DateTime;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::Mutex;

pub struct GoogleChatMessenger {
    name: String,
    mode: GoogleChatMode,
    client: Client,
    connected: bool,
}

enum GoogleChatMode {
    Webhook {
        webhook_url: String,
    },
    Api {
        token: String,
        space_id: String,
        api_base_url: String,
        last_seen_message_name: Mutex<Option<String>>,
    },
}

impl GoogleChatMessenger {
    pub fn new(name: impl Into<String>, webhook_url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            mode: GoogleChatMode::Webhook {
                webhook_url: webhook_url.into(),
            },
            client: Client::new(),
            connected: false,
        }
    }

    pub fn new_api(
        name: impl Into<String>,
        token: impl Into<String>,
        space_id: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            mode: GoogleChatMode::Api {
                token: token.into(),
                space_id: space_id.into(),
                api_base_url: "https://chat.googleapis.com/v1".to_string(),
                last_seen_message_name: Mutex::new(None),
            },
            client: Client::new(),
            connected: false,
        }
    }

    pub fn with_api_base_url(mut self, url: impl Into<String>) -> Self {
        if let GoogleChatMode::Api { api_base_url, .. } = &mut self.mode {
            *api_base_url = url.into();
        }
        self
    }

    fn api_url(api_base_url: &str, path: impl AsRef<str>) -> String {
        format!(
            "{}/{}",
            api_base_url.trim_end_matches('/'),
            path.as_ref().trim_start_matches('/')
        )
    }

    async fn api_get_json(&self, path: impl AsRef<str>) -> Result<Value> {
        let (token, api_base_url) = match &self.mode {
            GoogleChatMode::Api {
                token,
                api_base_url,
                ..
            } => (token, api_base_url),
            GoogleChatMode::Webhook { .. } => anyhow::bail!("Google Chat API requested in webhook mode"),
        };

        let response = self
            .client
            .get(Self::api_url(api_base_url, path))
            .bearer_auth(token)
            .send()
            .await
            .context("Google Chat API request failed")?;
        let status = response.status();
        let body = response
            .text()
            .await
            .context("Failed to read Google Chat API response body")?;

        if !status.is_success() {
            anyhow::bail!("Google Chat API request failed {}: {}", status, body);
        }

        serde_json::from_str(&body).context("Invalid Google Chat API response")
    }

    async fn api_post_json(&self, path: impl AsRef<str>, body: Value) -> Result<Value> {
        let (token, api_base_url) = match &self.mode {
            GoogleChatMode::Api {
                token,
                api_base_url,
                ..
            } => (token, api_base_url),
            GoogleChatMode::Webhook { .. } => anyhow::bail!("Google Chat API requested in webhook mode"),
        };

        let response = self
            .client
            .post(Self::api_url(api_base_url, path))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .context("Google Chat API request failed")?;
        let status = response.status();
        let response_body = response
            .text()
            .await
            .context("Failed to read Google Chat API response body")?;

        if !status.is_success() {
            anyhow::bail!("Google Chat API request failed {}: {}", status, response_body);
        }

        if response_body.trim().is_empty() {
            Ok(Value::Null)
        } else {
            serde_json::from_str(&response_body).context("Invalid Google Chat API response")
        }
    }

    fn space_path(space_id: &str) -> String {
        format!("spaces/{space_id}")
    }

    fn space_messages_path(space_id: &str) -> String {
        format!("spaces/{space_id}/messages")
    }

    async fn api_receive_messages(&self) -> Result<Vec<Message>> {
        let space_id = match &self.mode {
            GoogleChatMode::Api { space_id, .. } => space_id.clone(),
            GoogleChatMode::Webhook { .. } => return Ok(Vec::new()),
        };

        let last_seen = match &self.mode {
            GoogleChatMode::Api {
                last_seen_message_name,
                ..
            } => last_seen_message_name.lock().await.clone(),
            GoogleChatMode::Webhook { .. } => None,
        };

        let data = self.api_get_json(Self::space_messages_path(&space_id)).await?;
        let mut messages = Vec::new();
        let mut newest_name = last_seen.clone();

        if let Some(entries) = data["messages"].as_array() {
            let mut parsed = Vec::new();

            for entry in entries {
                let Some(name) = entry["name"].as_str() else {
                    continue;
                };
                let content = entry["text"].as_str().unwrap_or("").to_string();
                if content.is_empty() {
                    continue;
                }

                let timestamp = entry["createTime"]
                    .as_str()
                    .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                    .map(|value| value.timestamp())
                    .unwrap_or_else(|| chrono::Utc::now().timestamp());
                let sender = entry["sender"]["displayName"]
                    .as_str()
                    .or_else(|| entry["sender"]["name"].as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let is_direct = entry["space"]["type"].as_str() == Some("DM");

                parsed.push(Message {
                    id: name.to_string(),
                    sender,
                    content,
                    timestamp,
                    channel: Some(space_id.clone()),
                    reply_to: entry["thread"]["name"].as_str().map(ToString::to_string),
                    media: None,
                    is_direct,
                    reactions: None,
                });
            }

            if let Some(first) = parsed.first() {
                newest_name = Some(first.id.clone());
            }

            if let Some(seen_name) = &last_seen {
                for message in parsed {
                    if message.id == *seen_name {
                        break;
                    }
                    messages.push(message);
                }
                messages.reverse();
            } else {
                messages.extend(parsed.into_iter().rev());
            }
        }

        if let GoogleChatMode::Api {
            last_seen_message_name,
            ..
        } = &self.mode
        {
            *last_seen_message_name.lock().await = newest_name;
        }

        Ok(messages)
    }
}

#[async_trait]
impl Messenger for GoogleChatMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "googlechat"
    }

    async fn initialize(&mut self) -> Result<()> {
        if matches!(&self.mode, GoogleChatMode::Api { .. }) {
            let space_id = match &self.mode {
                GoogleChatMode::Api { space_id, .. } => space_id.clone(),
                GoogleChatMode::Webhook { .. } => String::new(),
            };
            self.api_get_json(Self::space_path(&space_id)).await?;
        }
        self.connected = true;
        Ok(())
    }

    async fn send_message(&self, space: &str, content: &str) -> Result<String> {
        match &self.mode {
            GoogleChatMode::Webhook { webhook_url } => {
                let body = json!({ "text": content });

                let resp = self.client.post(webhook_url).json(&body).send().await?;

                if resp.status().is_success() {
                    Ok(format!(
                        "googlechat:{}",
                        chrono::Utc::now().timestamp_millis()
                    ))
                } else {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    anyhow::bail!("Google Chat webhook failed {}: {}", status, text);
                }
            }
            GoogleChatMode::Api { space_id, .. } => {
                let target_space = if space.is_empty() { space_id } else { space };
                let data = self
                    .api_post_json(
                        Self::space_messages_path(target_space),
                        json!({ "text": content }),
                    )
                    .await?;

                Ok(data["name"].as_str().unwrap_or_default().to_string())
            }
        }
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        self.api_receive_messages().await
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let GoogleChatMode::Api {
            last_seen_message_name,
            ..
        } = &self.mode
        {
            *last_seen_message_name.lock().await = None;
        }
        self.connected = false;
        Ok(())
    }
}
