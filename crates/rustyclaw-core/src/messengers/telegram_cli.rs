//! Telegram messenger using Bot HTTP API.
//!
//! Uses the Telegram Bot API at https://api.telegram.org/bot<token>/
//! Simple REST-based implementation with no external dependencies.
//!
//! This requires the `telegram-cli` feature to be enabled.

use super::{Message, Messenger};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;

use std::sync::Arc;
use tokio::sync::Mutex;

/// Telegram API response wrapper
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TelegramResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

/// Telegram Update object
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Update {
    update_id: i64,
    message: Option<TelegramMessage>,
}

/// Telegram Message object
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TelegramMessage {
    message_id: i64,
    from: Option<TelegramUser>,
    chat: TelegramChat,
    date: i64,
    text: Option<String>,
}

/// Telegram User object
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TelegramUser {
    id: i64,
    first_name: String,
    last_name: Option<String>,
    username: Option<String>,
}

/// Telegram Chat object
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TelegramChat {
    id: i64,
    #[serde(rename = "type")]
    chat_type: String,
    title: Option<String>,
    username: Option<String>,
}

/// Telegram messenger implementation
pub struct TelegramCliMessenger {
    name: String,
    _token: String,
    client: Client,
    connected: Arc<Mutex<bool>>,
    last_update_id: Arc<Mutex<Option<i64>>>,
    base_url: String,
}

impl TelegramCliMessenger {
    /// Create a new Telegram messenger with bot token
    pub fn new(name: String, token: String) -> Self {
        let base_url = format!("https://api.telegram.org/bot{}", token);
        Self {
            name,
            _token: token,
            client: Client::new(),
            connected: Arc::new(Mutex::new(false)),
            last_update_id: Arc::new(Mutex::new(None)),
            base_url,
        }
    }

    /// Get bot info to verify token
    async fn get_me(&self) -> Result<Value> {
        let url = format!("{}/getMe", self.base_url);
        let response = self.client.get(&url).send().await?;
        
        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Telegram API error: {} - {}", status, error_text);
        }

        let data: TelegramResponse<Value> = response.json().await?;
        data.result.ok_or_else(|| anyhow::anyhow!("No result in response"))
    }

    /// Send a message to a chat
    async fn send_message_internal(&self, chat_id: &str, text: &str) -> Result<i64> {
        let url = format!("{}/sendMessage", self.base_url);
        
        let body = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "HTML"
        });

        let response = self.client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("Failed to send Telegram message")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to send message: {} - {}", status, error_text);
        }

        let data: TelegramResponse<TelegramMessage> = response.json().await?;
        Ok(data.result.map(|m| m.message_id).unwrap_or(0))
    }

    /// Get updates (new messages)
    async fn get_updates(&self, timeout: u64) -> Result<Vec<Update>> {
        let last_id = *self.last_update_id.lock().await;
        let mut url = format!("{}/getUpdates?timeout={}", self.base_url, timeout);
        
        if let Some(offset) = last_id {
            url.push_str(&format!("&offset={}", offset + 1));
        }

        let response = self.client.get(&url).send().await?;
        
        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to get updates: {} - {}", status, error_text);
        }

        let data: TelegramResponse<Vec<Update>> = response.json().await?;
        let updates = data.result.unwrap_or_default();

        // Update offset
        if let Some(last) = updates.last() {
            *self.last_update_id.lock().await = Some(last.update_id);
        }

        Ok(updates)
    }
}

impl std::fmt::Debug for TelegramCliMessenger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramCliMessenger")
            .field("name", &self.name)
            .field("connected", &self.connected)
            .finish()
    }
}

#[async_trait]
impl Messenger for TelegramCliMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "telegram"
    }

    async fn initialize(&mut self) -> Result<()> {
        // Verify token by calling getMe
        let _bot_info = self.get_me().await.context("Failed to verify Telegram bot token")?;
        *self.connected.lock().await = true;
        tracing::info!("Telegram bot connected successfully");
        Ok(())
    }

    async fn send_message(&self, recipient: &str, content: &str) -> Result<String> {
        let message_id = self.send_message_internal(recipient, content).await?;
        Ok(message_id.to_string())
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        let updates = self.get_updates(30).await?;
        
        let messages: Vec<Message> = updates
            .into_iter()
            .filter_map(|update| {
                update.message.map(|msg| {
                    let sender = msg.from
                        .map(|u| u.username.unwrap_or(u.first_name))
                        .unwrap_or_else(|| "unknown".to_string());
                    
                    Message {
                        id: msg.message_id.to_string(),
                        sender,
                        content: msg.text.unwrap_or_default(),
                        timestamp: msg.date,
                        channel: Some(msg.chat.id.to_string()),
                        reply_to: None,
                        media: None,
                    }
                })
            })
            .collect();

        Ok(messages)
    }

    fn is_connected(&self) -> bool {
        // Use try_lock to avoid blocking in sync context
        self.connected.try_lock().map(|g| *g).unwrap_or(false)
    }

    async fn disconnect(&mut self) -> Result<()> {
        *self.connected.lock().await = false;
        Ok(())
    }

    async fn set_typing(&self, channel: &str, typing: bool) -> Result<()> {
        if !typing {
            return Ok(()); // Telegram typing auto-expires, no need to clear
        }
        
        let url = format!("{}/sendChatAction", self.base_url);
        let body = serde_json::json!({
            "chat_id": channel,
            "action": "typing"
        });

        let response = self.client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("Failed to send typing indicator")?;

        if !response.status().is_success() {
            // Non-fatal, just log
            eprintln!("Failed to send Telegram typing indicator: {}", response.status());
        }

        Ok(())
    }
}
