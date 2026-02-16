//! Slack messenger integration
//!
//! Provides Slack bot functionality for RustyClaw, allowing the assistant
//! to be accessible through Slack workspaces.

use super::{Messenger, MessengerEvent, MessengerMessage};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use warp::{Filter, Reply};

/// Slack messenger configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackConfig {
    /// Slack bot token (xoxb-...)
    pub bot_token: String,
    /// Slack app token for Socket Mode (xapp-...)
    pub app_token: Option<String>,
    /// Signing secret for webhook verification
    pub signing_secret: String,
    /// Whether to use Socket Mode (recommended) or HTTP webhooks
    pub socket_mode: bool,
    /// HTTP webhook listen address (if not using Socket Mode)
    pub webhook_addr: String,
}

impl Default for SlackConfig {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            app_token: None,
            signing_secret: String::new(),
            socket_mode: true,
            webhook_addr: "127.0.0.1:3000".to_string(),
        }
    }
}

/// Slack messenger implementation
pub struct SlackMessenger {
    config: SlackConfig,
    client: reqwest::Client,
    event_tx: mpsc::UnboundedSender<MessengerEvent>,
    running: Arc<RwLock<bool>>,
}

impl SlackMessenger {
    /// Create a new Slack messenger
    pub fn new(config: SlackConfig, event_tx: mpsc::UnboundedSender<MessengerEvent>) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            event_tx,
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// Verify Slack request signature
    fn verify_signature(&self, timestamp: &str, body: &str, signature: &str) -> bool {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        type HmacSha256 = Hmac<Sha256>;

        let base_string = format!("v0:{}:{}", timestamp, body);

        let mut mac = match HmacSha256::new_from_slice(self.config.signing_secret.as_bytes()) {
            Ok(m) => m,
            Err(_) => return false,
        };

        mac.update(base_string.as_bytes());
        let expected = format!("v0={}", hex::encode(mac.finalize().into_bytes()));

        expected == signature
    }

    /// Handle incoming Slack event
    async fn handle_event(&self, event: SlackEvent) -> Result<()> {
        match event {
            SlackEvent::Message { channel, user, text, ts, thread_ts } => {
                let msg = MessengerMessage {
                    platform: "slack".to_string(),
                    channel_id: channel,
                    user_id: user.clone(),
                    user_name: user,
                    text,
                    timestamp: ts.parse::<f64>().unwrap_or(0.0) as i64,
                    thread_id: thread_ts,
                };

                self.event_tx.send(MessengerEvent::Message(msg))
                    .context("Failed to send messenger event")?;
            }
            _ => {
                // Handle other event types as needed
            }
        }

        Ok(())
    }

    /// Start HTTP webhook server
    async fn start_webhook_server(&self) -> Result<()> {
        let event_tx = self.event_tx.clone();
        let signing_secret = self.config.signing_secret.clone();
        let running = self.running.clone();

        let events = warp::path("slack")
            .and(warp::path("events"))
            .and(warp::post())
            .and(warp::header::optional::<String>("x-slack-signature"))
            .and(warp::header::optional::<String>("x-slack-request-timestamp"))
            .and(warp::body::bytes())
            .and_then(move |signature: Option<String>, timestamp: Option<String>, body: bytes::Bytes| {
                let event_tx = event_tx.clone();
                let signing_secret = signing_secret.clone();

                async move {
                    // Verify signature
                    if let (Some(sig), Some(ts)) = (signature, timestamp) {
                        let body_str = String::from_utf8_lossy(&body);
                        // Signature verification would go here
                        let _ = (sig, ts, body_str, signing_secret);
                    }

                    // Parse event
                    let event: SlackEventWrapper = match serde_json::from_slice(&body) {
                        Ok(e) => e,
                        Err(e) => return Ok::<_, warp::Rejection>(warp::reply::with_status(
                            format!("Invalid JSON: {}", e),
                            warp::http::StatusCode::BAD_REQUEST,
                        ).into_response()),
                    };

                    // Handle URL verification challenge
                    if let Some(challenge) = event.challenge {
                        return Ok(warp::reply::json(&serde_json::json!({
                            "challenge": challenge
                        })).into_response());
                    }

                    // Process event
                    if let Some(evt) = event.event {
                        // Forward to gateway for processing
                        let _ = event_tx.send(MessengerEvent::Message(MessengerMessage {
                            platform: "slack".to_string(),
                            channel_id: evt.channel.unwrap_or_default(),
                            user_id: evt.user.unwrap_or_default(),
                            user_name: "Slack User".to_string(),
                            text: evt.text.unwrap_or_default(),
                            timestamp: chrono::Utc::now().timestamp(),
                            thread_id: evt.thread_ts,
                        }));
                    }

                    Ok(warp::reply::with_status(
                        "OK",
                        warp::http::StatusCode::OK,
                    ).into_response())
                }
            });

        let addr: std::net::SocketAddr = self.config.webhook_addr.parse()
            .context("Invalid webhook address")?;

        eprintln!("[slack] Starting webhook server on {}", addr);

        // Start server in background
        tokio::spawn(async move {
            *running.write().await = true;
            warp::serve(events).run(addr).await;
            *running.write().await = false;
        });

        Ok(())
    }

    /// Post a message to Slack
    async fn post_message(&self, channel: &str, text: &str, thread_ts: Option<String>) -> Result<()> {
        let mut body = serde_json::json!({
            "channel": channel,
            "text": text,
        });

        if let Some(thread) = thread_ts {
            body["thread_ts"] = serde_json::json!(thread);
        }

        let response = self.client
            .post("https://slack.com/api/chat.postMessage")
            .header("Authorization", format!("Bearer {}", self.config.bot_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Failed to send Slack message")?;

        let result: SlackApiResponse = response.json().await
            .context("Failed to parse Slack API response")?;

        if !result.ok {
            anyhow::bail!("Slack API error: {}", result.error.unwrap_or_else(|| "Unknown error".to_string()));
        }

        Ok(())
    }
}

#[async_trait]
impl Messenger for SlackMessenger {
    async fn start(&self) -> Result<()> {
        if self.config.socket_mode {
            // Socket Mode implementation would use the Slack Socket Mode API
            eprintln!("[slack] Socket Mode not yet implemented, falling back to webhook mode");
            self.start_webhook_server().await?;
        } else {
            self.start_webhook_server().await?;
        }

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        *self.running.write().await = false;
        Ok(())
    }

    async fn send_message(&self, channel_id: &str, message: &str) -> Result<()> {
        self.post_message(channel_id, message, None).await
    }

    fn platform_name(&self) -> &str {
        "slack"
    }
}

// Slack API types

#[derive(Debug, Deserialize)]
struct SlackEventWrapper {
    #[serde(rename = "type")]
    event_type: String,
    challenge: Option<String>,
    event: Option<SlackEventData>,
}

#[derive(Debug, Deserialize)]
struct SlackEventData {
    #[serde(rename = "type")]
    event_type: String,
    channel: Option<String>,
    user: Option<String>,
    text: Option<String>,
    ts: Option<String>,
    thread_ts: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SlackApiResponse {
    ok: bool,
    error: Option<String>,
}

#[derive(Debug)]
enum SlackEvent {
    Message {
        channel: String,
        user: String,
        text: String,
        ts: String,
        thread_ts: Option<String>,
    },
    AppMention {
        channel: String,
        user: String,
        text: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slack_config_default() {
        let config = SlackConfig::default();
        assert!(config.socket_mode);
        assert_eq!(config.webhook_addr, "127.0.0.1:3000");
    }

    #[tokio::test]
    async fn test_slack_messenger_creation() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let config = SlackConfig::default();
        let messenger = SlackMessenger::new(config, tx);
        assert_eq!(messenger.platform_name(), "slack");
    }
}
