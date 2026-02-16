//! Gmail messenger implementation using Gmail API
//!
//! This implementation uses OAuth2 for authentication and polls the Gmail API
//! for new messages. It can also be configured to use Gmail Pub/Sub for
//! real-time notifications.
//!
//! Setup:
//! 1. Create OAuth2 credentials in Google Cloud Console
//! 2. Store credentials in vault: GMAIL_CLIENT_ID, GMAIL_CLIENT_SECRET
//! 3. Run oauth flow to get refresh token
//! 4. Configure in config.toml under [[messengers]]

use crate::messengers::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

/// Gmail API base URL
const GMAIL_API_BASE: &str = "https://www.googleapis.com/gmail/v1";

/// OAuth2 token endpoint
const OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// Gmail messenger configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailConfig {
    /// Client ID from Google Cloud Console
    pub client_id: String,
    /// Client secret from Google Cloud Console
    pub client_secret: String,
    /// Refresh token (obtained via OAuth2 flow)
    pub refresh_token: String,
    /// Email address to monitor (usually "me")
    #[serde(default = "GmailConfig::default_user")]
    pub user: String,
    /// Polling interval in seconds (default: 60)
    #[serde(default = "GmailConfig::default_poll_interval")]
    pub poll_interval: u64,
    /// Label to watch (default: "INBOX")
    #[serde(default = "GmailConfig::default_label")]
    pub label: String,
    /// Auto-reply only to unread messages
    #[serde(default = "GmailConfig::default_unread_only")]
    pub unread_only: bool,
}

impl GmailConfig {
    fn default_user() -> String {
        "me".to_string()
    }

    fn default_poll_interval() -> u64 {
        60 // Poll every 60 seconds
    }

    fn default_label() -> String {
        "INBOX".to_string()
    }

    fn default_unread_only() -> bool {
        true
    }
}

/// OAuth2 access token response
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: i64,
    #[serde(default)]
    refresh_token: Option<String>,
}

/// Cached access token
#[derive(Debug, Clone)]
struct AccessToken {
    token: String,
    expires_at: i64,
}

/// Gmail API message list response
#[derive(Debug, Deserialize)]
struct MessageList {
    #[serde(default)]
    messages: Vec<MessageRef>,
}

#[derive(Debug, Deserialize)]
struct MessageRef {
    id: String,
    #[serde(rename = "threadId")]
    thread_id: String,
}

/// Gmail API message detail
#[derive(Debug, Deserialize)]
struct GmailMessage {
    id: String,
    #[serde(rename = "threadId")]
    thread_id: String,
    #[serde(rename = "labelIds", default)]
    label_ids: Vec<String>,
    payload: MessagePayload,
    #[serde(rename = "internalDate")]
    internal_date: String,
}

#[derive(Debug, Deserialize)]
struct MessagePayload {
    headers: Vec<MessageHeader>,
    #[serde(default)]
    parts: Vec<MessagePart>,
    #[serde(default)]
    body: MessageBody,
}

#[derive(Debug, Deserialize)]
struct MessageHeader {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct MessagePart {
    #[serde(rename = "mimeType", default)]
    mime_type: String,
    #[serde(default)]
    body: MessageBody,
    #[serde(default)]
    parts: Vec<MessagePart>,
}

#[derive(Debug, Default, Deserialize)]
struct MessageBody {
    #[serde(default)]
    data: String,
}

/// Gmail messenger
pub struct GmailMessenger {
    config: GmailConfig,
    http: Client,
    access_token: Arc<RwLock<Option<AccessToken>>>,
    last_history_id: Arc<RwLock<Option<String>>>,
    processed_ids: Arc<RwLock<Vec<String>>>,
}

impl GmailMessenger {
    /// Create a new Gmail messenger
    pub fn new(config: GmailConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self {
            config,
            http,
            access_token: Arc::new(RwLock::new(None)),
            last_history_id: Arc::new(RwLock::new(None)),
            processed_ids: Arc::new(RwLock::new(Vec::new())),
        })
    }

    /// Get a valid access token (refresh if needed)
    async fn get_access_token(&self) -> Result<String> {
        let token_guard = self.access_token.read().await;

        // Check if we have a valid cached token
        if let Some(ref token) = *token_guard {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            if now < token.expires_at - 60 {
                // Token is still valid (with 60s buffer)
                return Ok(token.token.clone());
            }
        }
        drop(token_guard);

        // Need to refresh
        let resp: TokenResponse = self
            .http
            .post(OAUTH_TOKEN_URL)
            .form(&[
                ("client_id", self.config.client_id.as_str()),
                ("client_secret", self.config.client_secret.as_str()),
                ("refresh_token", self.config.refresh_token.as_str()),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await
            .context("Failed to request access token")?
            .json()
            .await
            .context("Failed to parse token response")?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let token = AccessToken {
            token: resp.access_token.clone(),
            expires_at: now + resp.expires_in,
        };

        *self.access_token.write().await = Some(token);

        Ok(resp.access_token)
    }

    /// List messages in inbox
    async fn list_messages(&self, max_results: usize) -> Result<Vec<MessageRef>> {
        let token = self.get_access_token().await?;

        // Build URL with query parameters manually (reqwest query() not available with our feature set)
        let mut url = format!(
            "{}/users/{}/messages?maxResults={}&labelIds={}",
            GMAIL_API_BASE,
            self.config.user,
            max_results,
            self.config.label
        );

        if self.config.unread_only {
            url.push_str("&q=is:unread");
        }

        let resp: MessageList = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .context("Failed to list messages")?
            .json()
            .await
            .context("Failed to parse message list")?;

        Ok(resp.messages)
    }

    /// Get message details
    async fn get_message(&self, id: &str) -> Result<GmailMessage> {
        let token = self.get_access_token().await?;
        let url = format!(
            "{}/users/{}/messages/{}?format=full",
            GMAIL_API_BASE, self.config.user, id
        );

        let msg: GmailMessage = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .context("Failed to get message")?
            .json()
            .await
            .context("Failed to parse message")?;

        Ok(msg)
    }

    /// Extract text from message body
    fn extract_text(payload: &MessagePayload) -> String {
        // Try to get text/plain part first
        if let Some(text) = Self::find_text_part(&payload.parts, "text/plain") {
            return text;
        }

        // Fallback to text/html
        if let Some(html) = Self::find_text_part(&payload.parts, "text/html") {
            return Self::strip_html(&html);
        }

        // Last resort: decode body directly
        if !payload.body.data.is_empty() {
            if let Ok(decoded) = URL_SAFE_NO_PAD.decode(&payload.body.data) {
                if let Ok(text) = String::from_utf8(decoded) {
                    return text;
                }
            }
        }

        String::new()
    }

    /// Find text part in message parts
    fn find_text_part(parts: &[MessagePart], mime_type: &str) -> Option<String> {
        for part in parts {
            if part.mime_type == mime_type && !part.body.data.is_empty() {
                if let Ok(decoded) = URL_SAFE_NO_PAD.decode(&part.body.data) {
                    if let Ok(text) = String::from_utf8(decoded) {
                        return Some(text);
                    }
                }
            }
            // Recursively search nested parts
            if !part.parts.is_empty() {
                if let Some(text) = Self::find_text_part(&part.parts, mime_type) {
                    return Some(text);
                }
            }
        }
        None
    }

    /// Basic HTML stripping (very simple, just removes tags)
    fn strip_html(html: &str) -> String {
        let mut result = String::new();
        let mut in_tag = false;

        for ch in html.chars() {
            match ch {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => result.push(ch),
                _ => {}
            }
        }

        result.trim().to_string()
    }

    /// Get header value
    fn get_header(msg: &GmailMessage, name: &str) -> Option<String> {
        msg.payload
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.clone())
    }

    /// Mark message as read
    async fn mark_as_read(&self, id: &str) -> Result<()> {
        let token = self.get_access_token().await?;
        let url = format!(
            "{}/users/{}/messages/{}/modify",
            GMAIL_API_BASE, self.config.user, id
        );

        let body = serde_json::json!({
            "removeLabelIds": ["UNREAD"]
        });

        self.http
            .post(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .context("Failed to mark message as read")?;

        Ok(())
    }

    /// Send an email
    async fn send_raw(&self, from: &str, to: &str, subject: &str, body: &str) -> Result<String> {
        let token = self.get_access_token().await?;
        let url = format!(
            "{}/users/{}/messages/send",
            GMAIL_API_BASE, self.config.user
        );

        // Build RFC 2822 email
        let email = format!(
            "From: {}\r\nTo: {}\r\nSubject: {}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{}",
            from, to, subject, body
        );

        // Base64 encode
        let encoded = URL_SAFE_NO_PAD.encode(email.as_bytes());

        let body = serde_json::json!({
            "raw": encoded
        });

        let resp: serde_json::Value = self
            .http
            .post(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .context("Failed to send email")?
            .json()
            .await
            .context("Failed to parse send response")?;

        Ok(resp["id"].as_str().unwrap_or("").to_string())
    }
}

#[async_trait]
impl Messenger for GmailMessenger {
    fn name(&self) -> &str {
        "gmail"
    }

    fn messenger_type(&self) -> &str {
        "gmail"
    }

    async fn initialize(&mut self) -> Result<()> {
        // Test authentication by getting a token
        self.get_access_token()
            .await
            .context("Failed to initialize Gmail messenger")?;

        eprintln!("[gmail] Initialized successfully");
        Ok(())
    }

    async fn send_message(&self, recipient: &str, content: &str) -> Result<String> {
        // Extract "from" address from config or use default
        let from = format!("{}@gmail.com", self.config.user);

        self.send_raw(&from, recipient, "RustyClaw Response", content)
            .await
    }

    async fn send_message_with_options(&self, opts: SendOptions<'_>) -> Result<String> {
        let from = format!("{}@gmail.com", self.config.user);

        // Use reply_to as subject hint if provided
        let subject = if let Some(reply_id) = opts.reply_to {
            format!("Re: {}", reply_id)
        } else {
            "RustyClaw Message".to_string()
        };

        self.send_raw(&from, opts.recipient, &subject, opts.content)
            .await
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        // List recent messages
        let message_refs = self.list_messages(10).await?;
        let mut messages = Vec::new();
        let mut processed = self.processed_ids.write().await;

        for msg_ref in message_refs {
            // Skip if already processed
            if processed.contains(&msg_ref.id) {
                continue;
            }

            // Get full message details
            let msg = match self.get_message(&msg_ref.id).await {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("[gmail] Failed to get message {}: {}", msg_ref.id, e);
                    continue;
                }
            };

            // Extract sender and subject
            let sender = Self::get_header(&msg, "From").unwrap_or_else(|| "unknown".to_string());
            let subject = Self::get_header(&msg, "Subject").unwrap_or_else(|| "(no subject)".to_string());

            // Extract body
            let body = Self::extract_text(&msg.payload);

            // Parse timestamp
            let timestamp = msg.internal_date.parse::<i64>().unwrap_or(0) / 1000;

            // Create message
            let message = Message {
                id: msg.id.clone(),
                sender: sender.clone(),
                content: format!("Subject: {}\n\n{}", subject, body),
                timestamp,
                channel: Some(self.config.label.clone()),
                reply_to: Some(msg.thread_id),
                media: None,
            };

            messages.push(message);

            // Mark as read if configured
            if self.config.unread_only {
                if let Err(e) = self.mark_as_read(&msg.id).await {
                    eprintln!("[gmail] Failed to mark message as read: {}", e);
                }
            }

            // Add to processed list (keep last 100)
            processed.push(msg.id);
            if processed.len() > 100 {
                processed.drain(0..50);
            }
        }

        Ok(messages)
    }

    fn is_connected(&self) -> bool {
        // Check if we have a valid token or can refresh
        !self.config.refresh_token.is_empty()
    }

    async fn disconnect(&mut self) -> Result<()> {
        *self.access_token.write().await = None;
        Ok(())
    }
}
