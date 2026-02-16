//! Microsoft Teams messenger.
//!
//! Supports:
//! - Incoming webhook mode (`webhook_url`)
//! - Microsoft Graph API mode (`token` + `team_id` + `channel_id`)
//!
//! Message polling uses Microsoft Graph API to fetch recent channel messages.

use crate::messengers::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

const TEAMS_GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";

/// Teams message from Graph API
#[derive(Debug, Clone, Deserialize)]
struct TeamsMessage {
    id: String,
    #[serde(default)]
    body: Option<TeamsMessageBody>,
    from: Option<TeamsMessageFrom>,
    #[serde(rename = "createdDateTime")]
    created_date_time: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TeamsMessageBody {
    content: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TeamsMessageFrom {
    user: Option<TeamsUser>,
}

#[derive(Debug, Clone, Deserialize)]
struct TeamsUser {
    id: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TeamsMessagesResponse {
    value: Vec<TeamsMessage>,
}

#[derive(Debug, Clone)]
pub struct TeamsConfig {
    pub token: Option<String>,
    pub webhook_url: Option<String>,
    pub team_id: Option<String>,
    pub channel_id: Option<String>,
    pub api_base: String,
}

impl Default for TeamsConfig {
    fn default() -> Self {
        Self {
            token: None,
            webhook_url: None,
            team_id: None,
            channel_id: None,
            api_base: TEAMS_GRAPH_BASE.to_string(),
        }
    }
}

pub struct TeamsMessenger {
    name: String,
    config: TeamsConfig,
    http: reqwest::Client,
    connected: bool,
    processed_ids: Arc<RwLock<HashSet<String>>>,
}

impl TeamsMessenger {
    pub fn new(name: String, config: TeamsConfig) -> Self {
        Self {
            name,
            config,
            http: reqwest::Client::new(),
            connected: false,
            processed_ids: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    fn teams_and_channel(&self, recipient: &str) -> Result<(String, String)> {
        if !recipient.trim().is_empty() {
            if let Some((team, channel)) = recipient.split_once('/') {
                return Ok((team.to_string(), channel.to_string()));
            }
            if let Some((team, channel)) = recipient.split_once(':') {
                return Ok((team.to_string(), channel.to_string()));
            }
        }

        let team_id = self
            .config
            .team_id
            .clone()
            .context("Teams requires team_id (or recipient as team/channel)")?;
        let channel_id = self
            .config
            .channel_id
            .clone()
            .context("Teams requires channel_id (or recipient as team/channel)")?;

        Ok((team_id, channel_id))
    }

    /// Fetch recent messages from a Teams channel
    async fn fetch_channel_messages(&self, team_id: &str, channel_id: &str) -> Result<Vec<TeamsMessage>> {
        let token = self
            .config
            .token
            .as_ref()
            .context("Teams API requires token")?;

        let url = format!(
            "{}/teams/{}/channels/{}/messages?$top=20",
            self.config.api_base, team_id, channel_id
        );

        let resp = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .context("Failed to fetch Teams messages")?;

        if !resp.status().is_success() {
            anyhow::bail!("Teams fetch messages failed: {}", resp.status());
        }

        let response: TeamsMessagesResponse = resp
            .json()
            .await
            .context("Failed to parse Teams messages response")?;

        Ok(response.value)
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
        if self.config.webhook_url.is_some() {
            self.connected = true;
            return Ok(());
        }

        let token = self
            .config
            .token
            .as_ref()
            .context("Teams API mode requires token")?;
        // If a default team is configured, perform a lightweight auth check.
        if let Some(team_id) = self.config.team_id.as_ref() {
            let url = format!("{}/teams/{}", self.config.api_base, team_id);
            let resp = self
                .http
                .get(url)
                .bearer_auth(token)
                .send()
                .await
                .context("Failed to contact Microsoft Graph API")?;

            if !resp.status().is_success() {
                anyhow::bail!("Teams auth failed: {}", resp.status());
            }
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
                .context("Failed to send Teams webhook message")?;

            if !resp.status().is_success() {
                anyhow::bail!("Teams webhook send failed: {}", resp.status());
            }

            return Ok(format!("teams-{}", chrono::Utc::now().timestamp_millis()));
        }

        let token = self
            .config
            .token
            .as_ref()
            .context("Teams token is not configured")?;
        let (team_id, channel_id) = self.teams_and_channel(opts.recipient)?;

        let url = format!(
            "{}/teams/{}/channels/{}/messages",
            self.config.api_base, team_id, channel_id
        );

        let resp = self
            .http
            .post(url)
            .bearer_auth(token)
            .json(&serde_json::json!({
                "body": {
                    "contentType": "text",
                    "content": opts.content,
                }
            }))
            .send()
            .await
            .context("Failed to send Teams API message")?;

        if !resp.status().is_success() {
            anyhow::bail!("Teams API send failed: {}", resp.status());
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Teams send response")?;

        Ok(data
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("teams-message")
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

        // Get team and channel IDs
        let team_id = match &self.config.team_id {
            Some(id) => id.clone(),
            None => return Ok(Vec::new()),
        };

        let channel_id = match &self.config.channel_id {
            Some(id) => id.clone(),
            None => return Ok(Vec::new()),
        };

        // Fetch recent messages
        let teams_messages = match self.fetch_channel_messages(&team_id, &channel_id).await {
            Ok(msgs) => msgs,
            Err(e) => {
                eprintln!("[teams] Failed to fetch messages: {}", e);
                return Ok(Vec::new());
            }
        };

        let mut new_messages = Vec::new();

        for msg in teams_messages {
            // Check if already processed
            {
                let mut processed = self.processed_ids.write().await;
                if processed.contains(&msg.id) {
                    continue;
                }
                processed.insert(msg.id.clone());

                // Keep only last 1000 IDs
                if processed.len() > 1000 {
                    let to_remove: Vec<_> = processed.iter().take(100).cloned().collect();
                    for id in to_remove {
                        processed.remove(&id);
                    }
                }
            }

            // Parse content
            let content = msg
                .body
                .as_ref()
                .and_then(|b| b.content.as_ref())
                .map(|c| c.clone())
                .unwrap_or_default();

            // Skip empty messages
            if content.trim().is_empty() {
                continue;
            }

            // Get sender info
            let sender = msg
                .from
                .as_ref()
                .and_then(|f| f.user.as_ref())
                .and_then(|u| u.display_name.as_ref())
                .map(|n| n.clone())
                .unwrap_or_else(|| "Unknown".to_string());

            // Parse timestamp
            let timestamp = chrono::DateTime::parse_from_rfc3339(&msg.created_date_time)
                .map(|dt| dt.timestamp())
                .unwrap_or_else(|_| chrono::Utc::now().timestamp());

            new_messages.push(Message {
                id: msg.id,
                sender,
                content,
                timestamp,
                channel: Some(format!("{}/{}", team_id, channel_id)),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_teams_type() {
        let m = TeamsMessenger::new("teams-main".to_string(), TeamsConfig::default());
        assert_eq!(m.messenger_type(), "teams");
    }
}
