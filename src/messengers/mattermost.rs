//! Mattermost messenger.
//!
//! Supports:
//! - Incoming webhook mode (`webhook_url`)
//! - REST API mode (`token` + `base_url` + channel id)
//!
//! Message polling uses Mattermost REST API to fetch recent channel posts.

use crate::messengers::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Mattermost post from API
#[derive(Debug, Clone, Deserialize)]
struct MattermostPost {
    id: String,
    channel_id: String,
    user_id: String,
    message: String,
    create_at: i64,
    #[serde(default)]
    root_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PostsResponse {
    #[serde(default)]
    order: Vec<String>,
    #[serde(default)]
    posts: HashMap<String, MattermostPost>,
}

#[derive(Debug, Deserialize)]
struct MattermostUser {
    id: String,
    username: String,
}

#[derive(Debug, Clone)]
pub struct MattermostConfig {
    pub token: Option<String>,
    pub webhook_url: Option<String>,
    pub base_url: String,
    pub default_channel: Option<String>,
}

impl Default for MattermostConfig {
    fn default() -> Self {
        Self {
            token: None,
            webhook_url: None,
            base_url: "http://localhost:8065".to_string(),
            default_channel: None,
        }
    }
}

pub struct MattermostMessenger {
    name: String,
    config: MattermostConfig,
    http: reqwest::Client,
    connected: bool,
    processed_ids: Arc<RwLock<HashSet<String>>>,
    user_cache: Arc<RwLock<HashMap<String, String>>>,
}

impl MattermostMessenger {
    pub fn new(name: String, config: MattermostConfig) -> Self {
        Self {
            name,
            config,
            http: reqwest::Client::new(),
            connected: false,
            processed_ids: Arc::new(RwLock::new(HashSet::new())),
            user_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn api_base(&self) -> String {
        self.config.base_url.trim_end_matches('/').to_string()
    }

    fn channel_for_send(&self, recipient: &str) -> Result<String> {
        if !recipient.trim().is_empty() {
            return Ok(recipient.to_string());
        }

        self.config
            .default_channel
            .clone()
            .context("Mattermost requires recipient channel id or default_channel")
    }

    /// Fetch recent posts from a channel
    async fn fetch_channel_posts(&self, channel_id: &str) -> Result<Vec<MattermostPost>> {
        let token = self
            .config
            .token
            .as_ref()
            .context("Mattermost API requires token")?;

        let url = format!(
            "{}/api/v4/channels/{}/posts?per_page=20",
            self.api_base(),
            channel_id
        );

        let resp = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .context("Failed to fetch Mattermost posts")?;

        if !resp.status().is_success() {
            anyhow::bail!("Mattermost fetch posts failed: {}", resp.status());
        }

        let response: PostsResponse = resp
            .json()
            .await
            .context("Failed to parse Mattermost posts response")?;

        // Convert posts in order
        let mut posts = Vec::new();
        for post_id in &response.order {
            if let Some(post) = response.posts.get(post_id) {
                posts.push(post.clone());
            }
        }

        Ok(posts)
    }

    /// Get username for a user ID (with caching)
    async fn get_username(&self, user_id: &str) -> String {
        // Check cache first
        {
            let cache = self.user_cache.read().await;
            if let Some(username) = cache.get(user_id) {
                return username.clone();
            }
        }

        // Fetch from API
        let token = match self.config.token.as_ref() {
            Some(t) => t,
            None => return user_id.to_string(),
        };

        let url = format!("{}/api/v4/users/{}", self.api_base(), user_id);

        let resp = match self.http.get(&url).bearer_auth(token).send().await {
            Ok(r) => r,
            Err(_) => return user_id.to_string(),
        };

        if !resp.status().is_success() {
            return user_id.to_string();
        }

        let user: Result<MattermostUser, _> = resp.json().await;
        let username = user
            .map(|u| u.username)
            .unwrap_or_else(|_| user_id.to_string());

        // Cache it
        {
            let mut cache = self.user_cache.write().await;
            cache.insert(user_id.to_string(), username.clone());
        }

        username
    }
}

#[async_trait]
impl Messenger for MattermostMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "mattermost"
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
            .context("Mattermost API mode requires token")?;
        let url = format!("{}/api/v4/users/me", self.api_base());

        let resp = self
            .http
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .context("Failed to contact Mattermost API")?;

        if !resp.status().is_success() {
            anyhow::bail!("Mattermost auth failed: {}", resp.status());
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
            let mut payload = serde_json::json!({ "text": opts.content });
            let target = if opts.recipient.is_empty() {
                self.config.default_channel.as_deref()
            } else {
                Some(opts.recipient)
            };
            if let Some(channel) = target {
                payload["channel"] = serde_json::Value::String(channel.to_string());
            }

            let resp = self
                .http
                .post(webhook_url)
                .json(&payload)
                .send()
                .await
                .context("Failed to send Mattermost webhook message")?;

            if !resp.status().is_success() {
                anyhow::bail!("Mattermost webhook send failed: {}", resp.status());
            }

            return Ok(format!(
                "mattermost-{}",
                chrono::Utc::now().timestamp_millis()
            ));
        }

        let token = self
            .config
            .token
            .as_ref()
            .context("Mattermost token is not configured")?;
        let channel_id = self.channel_for_send(opts.recipient)?;
        let url = format!("{}/api/v4/posts", self.api_base());

        let mut payload = serde_json::json!({
            "channel_id": channel_id,
            "message": opts.content,
        });
        if let Some(reply_to) = opts.reply_to {
            payload["root_id"] = serde_json::Value::String(reply_to.to_string());
        }

        let resp = self
            .http
            .post(url)
            .bearer_auth(token)
            .json(&payload)
            .send()
            .await
            .context("Failed to send Mattermost API message")?;

        if !resp.status().is_success() {
            anyhow::bail!("Mattermost API send failed: {}", resp.status());
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Mattermost send response")?;

        Ok(data
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("mattermost-message")
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

        // Get channel ID
        let channel_id = match &self.config.default_channel {
            Some(id) => id.clone(),
            None => return Ok(Vec::new()),
        };

        // Fetch recent posts
        let posts = match self.fetch_channel_posts(&channel_id).await {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[mattermost] Failed to fetch posts: {}", e);
                return Ok(Vec::new());
            }
        };

        let mut new_messages = Vec::new();

        for post in posts {
            // Check if already processed
            {
                let mut processed = self.processed_ids.write().await;
                if processed.contains(&post.id) {
                    continue;
                }
                processed.insert(post.id.clone());

                // Keep only last 1000 IDs
                if processed.len() > 1000 {
                    let to_remove: Vec<_> = processed.iter().take(100).cloned().collect();
                    for id in to_remove {
                        processed.remove(&id);
                    }
                }
            }

            // Skip empty messages
            if post.message.trim().is_empty() {
                continue;
            }

            // Get username
            let username = self.get_username(&post.user_id).await;

            // Convert timestamp from milliseconds to seconds
            let timestamp = post.create_at / 1000;

            new_messages.push(Message {
                id: post.id,
                sender: username,
                content: post.message,
                timestamp,
                channel: Some(post.channel_id),
                reply_to: post.root_id,
                media: None,
            });
        }

        // Return newest first (reverse chronological) - posts are already in order
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
    fn test_mattermost_type() {
        let m = MattermostMessenger::new("mm-main".to_string(), MattermostConfig::default());
        assert_eq!(m.messenger_type(), "mattermost");
    }
}
