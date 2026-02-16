//! Telegram messenger integration
//!
//! Provides Telegram bot functionality for RustyClaw, allowing the assistant
//! to be accessible through Telegram chats.

use super::{Messenger, MessengerEvent, MessengerMessage};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{sleep, Duration};

/// Telegram messenger configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    /// Telegram bot token (from @BotFather)
    pub bot_token: String,
    /// Webhook URL (if using webhooks instead of long polling)
    pub webhook_url: Option<String>,
    /// Webhook listen address (if using webhooks)
    pub webhook_addr: Option<String>,
    /// Poll interval in seconds (for long polling mode)
    pub poll_interval_secs: u64,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            webhook_url: None,
            webhook_addr: Some("127.0.0.1:8443".to_string()),
            poll_interval_secs: 1,
        }
    }
}

/// Telegram messenger implementation
pub struct TelegramMessenger {
    config: TelegramConfig,
    client: reqwest::Client,
    event_tx: mpsc::UnboundedSender<MessengerEvent>,
    running: Arc<RwLock<bool>>,
    last_update_id: Arc<RwLock<i64>>,
}

impl TelegramMessenger {
    /// Create a new Telegram messenger
    pub fn new(config: TelegramConfig, event_tx: mpsc::UnboundedSender<MessengerEvent>) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            event_tx,
            running: Arc::new(RwLock::new(false)),
            last_update_id: Arc::new(RwLock::new(0)),
        }
    }

    /// Get Telegram Bot API URL
    fn api_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{}", self.config.bot_token, method)
    }

    /// Start long polling for updates
    async fn start_long_polling(&self) -> Result<()> {
        *self.running.write().await = true;

        let client = self.client.clone();
        let event_tx = self.event_tx.clone();
        let running = self.running.clone();
        let last_update_id = self.last_update_id.clone();
        let api_url = self.api_url("getUpdates");
        let poll_interval = self.config.poll_interval_secs;

        tokio::spawn(async move {
            eprintln!("[telegram] Starting long polling...");

            while *running.read().await {
                let offset = *last_update_id.read().await + 1;

                let response = match client
                    .post(&api_url)
                    .json(&serde_json::json!({
                        "offset": offset,
                        "timeout": 30,
                        "allowed_updates": ["message", "edited_message"]
                    }))
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("[telegram] Poll error: {}", e);
                        sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                };

                let result: TelegramResponse<Vec<TelegramUpdate>> = match response.json().await {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("[telegram] Parse error: {}", e);
                        sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                };

                if !result.ok {
                    eprintln!("[telegram] API error: {:?}", result.description);
                    sleep(Duration::from_secs(5)).await;
                    continue;
                }

                if let Some(updates) = result.result {
                    for update in updates {
                        *last_update_id.write().await = update.update_id;

                        if let Some(message) = update.message {
                            let msg = MessengerMessage {
                                platform: "telegram".to_string(),
                                channel_id: message.chat.id.to_string(),
                                user_id: message.from.as_ref().map(|u| u.id.to_string()).unwrap_or_default(),
                                user_name: message.from.as_ref()
                                    .map(|u| u.username.clone().unwrap_or(format!("{} {}", u.first_name, u.last_name.as_deref().unwrap_or(""))))
                                    .unwrap_or_default(),
                                text: message.text.unwrap_or_default(),
                                timestamp: message.date,
                                thread_id: message.message_thread_id.map(|id| id.to_string()),
                            };

                            let _ = event_tx.send(MessengerEvent::Message(msg));
                        }
                    }
                }

                sleep(Duration::from_secs(poll_interval)).await;
            }

            eprintln!("[telegram] Long polling stopped");
        });

        Ok(())
    }

    /// Send a message to Telegram chat
    async fn post_message(&self, chat_id: &str, text: &str, reply_to_message_id: Option<i64>) -> Result<()> {
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "Markdown",
        });

        if let Some(reply_id) = reply_to_message_id {
            body["reply_to_message_id"] = serde_json::json!(reply_id);
        }

        let response = self.client
            .post(self.api_url("sendMessage"))
            .json(&body)
            .send()
            .await
            .context("Failed to send Telegram message")?;

        let result: TelegramResponse<serde_json::Value> = response.json().await
            .context("Failed to parse Telegram API response")?;

        if !result.ok {
            anyhow::bail!("Telegram API error: {}", result.description.unwrap_or_else(|| "Unknown error".to_string()));
        }

        Ok(())
    }

    /// Send a photo to Telegram chat
    pub async fn send_photo(&self, chat_id: &str, photo_url: &str, caption: Option<&str>) -> Result<()> {
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "photo": photo_url,
        });

        if let Some(cap) = caption {
            body["caption"] = serde_json::json!(cap);
        }

        let response = self.client
            .post(self.api_url("sendPhoto"))
            .json(&body)
            .send()
            .await
            .context("Failed to send Telegram photo")?;

        let result: TelegramResponse<serde_json::Value> = response.json().await
            .context("Failed to parse Telegram API response")?;

        if !result.ok {
            anyhow::bail!("Telegram API error: {}", result.description.unwrap_or_else(|| "Unknown error".to_string()));
        }

        Ok(())
    }

    /// Get bot info
    pub async fn get_me(&self) -> Result<TelegramUser> {
        let response = self.client
            .get(self.api_url("getMe"))
            .send()
            .await
            .context("Failed to get bot info")?;

        let result: TelegramResponse<TelegramUser> = response.json().await
            .context("Failed to parse bot info response")?;

        if !result.ok {
            anyhow::bail!("Telegram API error: {}", result.description.unwrap_or_else(|| "Unknown error".to_string()));
        }

        result.result.context("No bot info in response")
    }
}

#[async_trait]
impl Messenger for TelegramMessenger {
    async fn start(&self) -> Result<()> {
        // Verify bot token by getting bot info
        match self.get_me().await {
            Ok(bot_info) => {
                eprintln!("[telegram] Connected as @{}", bot_info.username.unwrap_or_else(|| bot_info.first_name.clone()));
            }
            Err(e) => {
                anyhow::bail!("Failed to verify Telegram bot token: {}", e);
            }
        }

        if self.config.webhook_url.is_some() {
            // TODO: Implement webhook mode
            eprintln!("[telegram] Webhook mode not yet implemented, using long polling");
            self.start_long_polling().await?;
        } else {
            self.start_long_polling().await?;
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
        "telegram"
    }
}

// Telegram API types

#[derive(Debug, Deserialize)]
struct TelegramResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    update_id: i64,
    message: Option<TelegramMessage>,
    edited_message: Option<TelegramMessage>,
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    message_id: i64,
    from: Option<TelegramUser>,
    chat: TelegramChat,
    date: i64,
    text: Option<String>,
    message_thread_id: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TelegramUser {
    pub id: i64,
    pub is_bot: bool,
    pub first_name: String,
    pub last_name: Option<String>,
    pub username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramChat {
    id: i64,
    #[serde(rename = "type")]
    chat_type: String,
    title: Option<String>,
    username: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telegram_config_default() {
        let config = TelegramConfig::default();
        assert_eq!(config.poll_interval_secs, 1);
        assert_eq!(config.webhook_addr, Some("127.0.0.1:8443".to_string()));
    }

    #[tokio::test]
    async fn test_telegram_messenger_creation() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let config = TelegramConfig::default();
        let messenger = TelegramMessenger::new(config, tx);
        assert_eq!(messenger.platform_name(), "telegram");
    }

    #[test]
    fn test_api_url() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let config = TelegramConfig {
            bot_token: "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11".to_string(),
            ..Default::default()
        };
        let messenger = TelegramMessenger::new(config, tx);
        assert_eq!(
            messenger.api_url("getMe"),
            "https://api.telegram.org/bot123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11/getMe"
        );
    }
}
