//! Microsoft Teams messenger — Incoming Webhook and Microsoft Graph implementation.

use crate::{Message, Messenger};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::DateTime;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::Mutex;

pub struct TeamsMessenger {
    name: String,
    mode: TeamsMode,
    client: Client,
    connected: bool,
}

enum TeamsMode {
    Webhook {
        webhook_url: String,
    },
    Graph {
        token: String,
        team_id: String,
        channel_id: String,
        graph_base_url: String,
        last_seen_message_id: Mutex<Option<String>>,
    },
}

impl TeamsMessenger {
    pub fn new(name: impl Into<String>, webhook_url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            mode: TeamsMode::Webhook {
                webhook_url: webhook_url.into(),
            },
            client: Client::new(),
            connected: false,
        }
    }

    pub fn new_graph(
        name: impl Into<String>,
        token: impl Into<String>,
        team_id: impl Into<String>,
        channel_id: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            mode: TeamsMode::Graph {
                token: token.into(),
                team_id: team_id.into(),
                channel_id: channel_id.into(),
                graph_base_url: "https://graph.microsoft.com/v1.0".to_string(),
                last_seen_message_id: Mutex::new(None),
            },
            client: Client::new(),
            connected: false,
        }
    }

    pub fn with_graph_base_url(mut self, url: impl Into<String>) -> Self {
        if let TeamsMode::Graph { graph_base_url, .. } = &mut self.mode {
            *graph_base_url = url.into();
        }
        self
    }

    fn graph_api_url(graph_base_url: &str, path: impl AsRef<str>) -> String {
        format!(
            "{}/{}",
            graph_base_url.trim_end_matches('/'),
            path.as_ref().trim_start_matches('/')
        )
    }

    async fn graph_get_json(&self, path: impl AsRef<str>) -> Result<Value> {
        let (token, graph_base_url) = match &self.mode {
            TeamsMode::Graph {
                token,
                graph_base_url,
                ..
            } => (token, graph_base_url),
            TeamsMode::Webhook { .. } => anyhow::bail!("Teams Graph API requested in webhook mode"),
        };

        let response = self
            .client
            .get(Self::graph_api_url(graph_base_url, path))
            .bearer_auth(token)
            .send()
            .await
            .context("Teams Graph request failed")?;
        let status = response.status();
        let body = response
            .text()
            .await
            .context("Failed to read Teams Graph response body")?;

        if !status.is_success() {
            anyhow::bail!("Teams Graph request failed {}: {}", status, body);
        }

        serde_json::from_str(&body).context("Invalid Teams Graph response")
    }

    async fn graph_post_json(&self, path: impl AsRef<str>, body: Value) -> Result<Value> {
        let (token, graph_base_url) = match &self.mode {
            TeamsMode::Graph {
                token,
                graph_base_url,
                ..
            } => (token, graph_base_url),
            TeamsMode::Webhook { .. } => anyhow::bail!("Teams Graph API requested in webhook mode"),
        };

        let response = self
            .client
            .post(Self::graph_api_url(graph_base_url, path))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .context("Teams Graph request failed")?;
        let status = response.status();
        let response_body = response
            .text()
            .await
            .context("Failed to read Teams Graph response body")?;

        if !status.is_success() {
            anyhow::bail!("Teams Graph request failed {}: {}", status, response_body);
        }

        if response_body.trim().is_empty() {
            Ok(Value::Null)
        } else {
            serde_json::from_str(&response_body).context("Invalid Teams Graph response")
        }
    }

    fn graph_messages_path(team_id: &str, channel_id: &str) -> String {
        format!("teams/{team_id}/channels/{channel_id}/messages")
    }

    async fn graph_receive_messages(&self) -> Result<Vec<Message>> {
        let (team_id, channel_id) = match &self.mode {
            TeamsMode::Graph {
                team_id,
                channel_id,
                ..
            } => (team_id.clone(), channel_id.clone()),
            TeamsMode::Webhook { .. } => return Ok(Vec::new()),
        };

        let last_seen = match &self.mode {
            TeamsMode::Graph {
                last_seen_message_id,
                ..
            } => last_seen_message_id.lock().await.clone(),
            TeamsMode::Webhook { .. } => None,
        };

        let data = self
            .graph_get_json(Self::graph_messages_path(&team_id, &channel_id))
            .await?;

        let mut messages = Vec::new();
        let mut newest_id = last_seen.clone();

        if let Some(entries) = data["value"].as_array() {
            let mut parsed = Vec::new();

            for entry in entries {
                let id = entry["id"].as_str().unwrap_or_default().to_string();
                let body_content = entry["body"]["content"].as_str().unwrap_or("").to_string();
                if body_content.is_empty() {
                    continue;
                }

                let sender = entry["from"]["user"]["displayName"]
                    .as_str()
                    .or_else(|| entry["from"]["application"]["displayName"].as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let timestamp = entry["createdDateTime"]
                    .as_str()
                    .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                    .map(|value| value.timestamp())
                    .unwrap_or_else(|| chrono::Utc::now().timestamp());
                let reply_to = entry["replyToId"].as_str().map(ToString::to_string);

                parsed.push(Message {
                    id,
                    sender,
                    content: body_content,
                    timestamp,
                    channel: Some(channel_id.clone()),
                    reply_to,
                    media: None,
                    is_direct: false,
                    reactions: None,
                });
            }

            if let Some(first) = parsed.first() {
                newest_id = Some(first.id.clone());
            }

            if let Some(seen_id) = &last_seen {
                for message in parsed {
                    if message.id == *seen_id {
                        break;
                    }
                    messages.push(message);
                }
                messages.reverse();
            } else {
                messages.extend(parsed.into_iter().rev());
            }
        }

        if let TeamsMode::Graph {
            last_seen_message_id,
            ..
        } = &self.mode
        {
            *last_seen_message_id.lock().await = newest_id;
        }

        Ok(messages)
    }
}

#[async_trait]
impl Messenger for TeamsMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "msteams"
    }

    async fn initialize(&mut self) -> Result<()> {
        if matches!(&self.mode, TeamsMode::Graph { .. }) {
            self.graph_get_json("me").await?;
        }
        self.connected = true;
        Ok(())
    }

    async fn send_message(&self, channel: &str, content: &str) -> Result<String> {
        match &self.mode {
            TeamsMode::Webhook { webhook_url } => {
                let body = json!({
                    "@type": "MessageCard",
                    "@context": "https://schema.org/extensions",
                    "text": content,
                });

                let resp = self.client.post(webhook_url).json(&body).send().await?;

                if resp.status().is_success() {
                    Ok(format!("teams:{}", chrono::Utc::now().timestamp_millis()))
                } else {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    anyhow::bail!("Teams webhook failed {}: {}", status, text);
                }
            }
            TeamsMode::Graph {
                team_id,
                channel_id,
                ..
            } => {
                let path = if channel.is_empty() {
                    Self::graph_messages_path(team_id, channel_id)
                } else {
                    Self::graph_messages_path(team_id, channel)
                };
                let data = self
                    .graph_post_json(path, json!({
                        "body": {
                            "contentType": "html",
                            "content": content,
                        }
                    }))
                    .await?;

                Ok(data["id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string())
            }
        }
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        self.graph_receive_messages().await
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let TeamsMode::Graph {
            last_seen_message_id,
            ..
        } = &self.mode
        {
            *last_seen_message_id.lock().await = None;
        }
        self.connected = false;
        Ok(())
    }
}
