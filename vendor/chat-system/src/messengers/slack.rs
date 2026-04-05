//! Slack messenger — Web API implementation.

use crate::{Message, Messenger};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::Mutex;

pub struct SlackMessenger {
    name: String,
    token: String,
    api_base_url: String,
    client: Client,
    last_seen_ts: Mutex<HashMap<String, String>>,
    connected: bool,
}

impl SlackMessenger {
    pub fn new(name: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            token: token.into(),
            api_base_url: "https://slack.com/api".to_string(),
            client: Client::new(),
            last_seen_ts: Mutex::new(HashMap::new()),
            connected: false,
        }
    }

    pub fn with_api_base_url(mut self, url: impl Into<String>) -> Self {
        self.api_base_url = url.into();
        self
    }

    fn api_url(&self, path: impl AsRef<str>) -> String {
        format!(
            "{}/{}",
            self.api_base_url.trim_end_matches('/'),
            path.as_ref().trim_start_matches('/')
        )
    }

    async fn get_json(&self, path: impl AsRef<str>) -> Result<Value> {
        let response = self
            .client
            .get(self.api_url(path))
            .bearer_auth(&self.token)
            .send()
            .await
            .context("Slack API request failed")?;

        let status = response.status();
        let body = response.text().await.context("Failed to read Slack response body")?;
        if !status.is_success() {
            anyhow::bail!("Slack API request failed {}: {}", status, body);
        }

        serde_json::from_str(&body).context("Invalid Slack API response")
    }

    async fn post_json(&self, path: impl AsRef<str>, body: Value) -> Result<Value> {
        let response = self
            .client
            .post(self.api_url(path))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .context("Slack API request failed")?;

        let status = response.status();
        let response_body = response.text().await.context("Failed to read Slack response body")?;
        if !status.is_success() {
            anyhow::bail!("Slack API request failed {}: {}", status, response_body);
        }

        serde_json::from_str(&response_body).context("Invalid Slack API response")
    }

    fn parse_ok_response(&self, data: &Value, operation: &str) -> Result<()> {
        if data["ok"].as_bool().unwrap_or(false) {
            Ok(())
        } else {
            anyhow::bail!("Slack {} failed: {:?}", operation, data);
        }
    }

    async fn fetch_conversation_ids(&self) -> Result<Vec<String>> {
        let data = self
            .get_json("conversations.list?types=public_channel,private_channel,im,mpim&exclude_archived=true&limit=1000")
            .await?;
        self.parse_ok_response(&data, "conversations.list")?;

        Ok(data["channels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|channel| channel["id"].as_str().map(ToString::to_string))
            .collect())
    }

    async fn fetch_channel_messages(
        &self,
        channel_id: &str,
        last_seen_ts: Option<&str>,
    ) -> Result<Vec<(String, Message)>> {
        let mut path = format!("conversations.history?channel={channel_id}&limit=100");
        if let Some(ts) = last_seen_ts {
            path.push_str("&oldest=");
            path.push_str(ts);
            path.push_str("&inclusive=false");
        }

        let data = self.get_json(path).await?;
        self.parse_ok_response(&data, "conversations.history")?;

        let mut messages = Vec::new();
        if let Some(entries) = data["messages"].as_array() {
            for entry in entries.iter().rev() {
                let Some(ts) = entry["ts"].as_str() else {
                    continue;
                };

                let content = entry["text"].as_str().unwrap_or("").to_string();
                if content.is_empty() && entry.get("files").is_none() {
                    continue;
                }

                let sender = entry["user"]
                    .as_str()
                    .or_else(|| entry["bot_id"].as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let timestamp = ts
                    .split('.')
                    .next()
                    .and_then(|seconds| seconds.parse::<i64>().ok())
                    .unwrap_or_default();

                messages.push((
                    ts.to_string(),
                    Message {
                        id: ts.to_string(),
                        sender,
                        content,
                        timestamp,
                        channel: Some(channel_id.to_string()),
                        reply_to: entry["thread_ts"]
                            .as_str()
                            .filter(|thread_ts| *thread_ts != ts)
                            .map(ToString::to_string),
                        media: None,
                        is_direct: false,
                        reactions: None,
                    },
                ));
            }
        }

        Ok(messages)
    }
}

#[async_trait]
impl Messenger for SlackMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "slack"
    }

    async fn initialize(&mut self) -> Result<()> {
        let data = self.get_json("auth.test").await?;
        self.parse_ok_response(&data, "auth.test")?;
        self.connected = true;
        Ok(())
    }

    async fn send_message(&self, channel: &str, text: &str) -> Result<String> {
        let data = self
            .post_json(
                "chat.postMessage",
                json!({
                "channel": channel,
                "text": text,
                }),
            )
            .await?;
        self.parse_ok_response(&data, "chat.postMessage")?;

        Ok(data["ts"].as_str().unwrap_or("").to_string())
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        let conversation_ids = self.fetch_conversation_ids().await?;
        let mut received = Vec::new();

        for channel_id in conversation_ids {
            let channel_last_seen = {
                let last_seen = self.last_seen_ts.lock().await;
                last_seen.get(&channel_id).cloned()
            };
            let channel_messages = self
                .fetch_channel_messages(&channel_id, channel_last_seen.as_deref())
                .await?;

            if let Some((latest_ts, _)) = channel_messages.last() {
                let mut last_seen = self.last_seen_ts.lock().await;
                last_seen.insert(channel_id.clone(), latest_ts.clone());
            }

            received.extend(channel_messages.into_iter().map(|(_, message)| message));
        }

        Ok(received)
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.last_seen_ts.lock().await.clear();
        self.connected = false;
        Ok(())
    }
}
