//! Microsoft Teams messenger.
//!
//! Supports:
//! - Incoming webhook mode (`webhook_url`)
//! - Microsoft Graph API mode (`token` + `team_id` + `channel_id`)
//!
//! Inbound message polling is not implemented and returns an empty list.

use crate::messengers::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;

const TEAMS_GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";

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
}

impl TeamsMessenger {
    pub fn new(name: String, config: TeamsConfig) -> Self {
        Self {
            name,
            config,
            http: reqwest::Client::new(),
            connected: false,
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
    fn test_teams_type() {
        let m = TeamsMessenger::new("teams-main".to_string(), TeamsConfig::default());
        assert_eq!(m.messenger_type(), "teams");
    }
}
