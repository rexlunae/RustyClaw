//! Telegram messenger — Bot API polling implementation.

use crate::{Message, Messenger};
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use tokio::sync::Mutex;

pub struct TelegramMessenger {
    name: String,
    api_base_url: String,
    client: Client,
    last_update_id: Mutex<Option<i64>>,
    connected: bool,
}

impl TelegramMessenger {
    pub fn new(name: impl Into<String>, token: impl Into<String>) -> Self {
        let token = token.into();
        Self {
            name: name.into(),
            api_base_url: format!("https://api.telegram.org/bot{token}"),
            client: Client::new(),
            last_update_id: Mutex::new(None),
            connected: false,
        }
    }

    pub fn with_api_base_url(mut self, url: impl Into<String>) -> Self {
        self.api_base_url = url.into();
        self
    }

    fn api_url(&self, method: impl AsRef<str>) -> String {
        format!("{}/{}", self.api_base_url.trim_end_matches('/'), method.as_ref())
    }

    fn get_updates_url(&self, offset: Option<i64>) -> String {
        match offset {
            Some(offset) => format!("{}?offset={offset}&timeout=0", self.api_url("getUpdates")),
            None => format!("{}?timeout=0", self.api_url("getUpdates")),
        }
    }
}

#[async_trait]
impl Messenger for TelegramMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "telegram"
    }

    async fn initialize(&mut self) -> Result<()> {
        let resp = self.client.get(self.api_url("getMe")).send().await?;

        let data: Value = resp.json().await?;
        if data["ok"].as_bool().unwrap_or(false) {
            self.connected = true;
            Ok(())
        } else {
            anyhow::bail!("Telegram getMe failed: {:?}", data);
        }
    }

    async fn send_message(&self, chat_id: &str, text: &str) -> Result<String> {
        let resp = self
            .client
            .post(self.api_url("sendMessage"))
            .json(&serde_json::json!({
                "chat_id": chat_id,
                "text": text,
                "parse_mode": "HTML",
            }))
            .send()
            .await?;

        let data: Value = resp.json().await?;
        if data["ok"].as_bool().unwrap_or(false) {
            let id = data["result"]["message_id"]
                .as_i64()
                .map(|i| i.to_string())
                .unwrap_or_default();
            Ok(id)
        } else {
            anyhow::bail!("Telegram sendMessage failed: {:?}", data);
        }
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        let next_offset = {
            let last_update_id = self.last_update_id.lock().await;
            last_update_id.map(|update_id| update_id + 1)
        };
        let resp = self.client.get(self.get_updates_url(next_offset)).send().await?;

        let data: Value = resp.json().await?;
        let mut messages = Vec::new();
        let mut max_update_id: Option<i64> = None;

        if let Some(updates) = data["result"].as_array() {
            for update in updates {
                if let Some(update_id) = update["update_id"].as_i64() {
                    max_update_id = Some(match max_update_id {
                        Some(current) => current.max(update_id),
                        None => update_id,
                    });
                }

                if let Some(msg) = update.get("message") {
                    let id = msg["message_id"].as_i64().unwrap_or(0).to_string();
                    let sender = msg["from"]["username"]
                        .as_str()
                        .or_else(|| msg["from"]["first_name"].as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let content = msg["text"].as_str().unwrap_or("").to_string();
                    let timestamp = msg["date"].as_i64().unwrap_or(0);
                    let chat_id = msg["chat"]["id"].as_i64().map(|i| i.to_string());

                    messages.push(Message {
                        id,
                        sender,
                        content,
                        timestamp,
                        channel: chat_id,
                        reply_to: None,
                        media: None,
                        is_direct: false,
                        reactions: None,
                    });
                }
            }
        }

        if let Some(max_update_id) = max_update_id {
            *self.last_update_id.lock().await = Some(max_update_id);
        }

        Ok(messages)
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    async fn disconnect(&mut self) -> Result<()> {
        *self.last_update_id.lock().await = None;
        self.connected = false;
        Ok(())
    }
}
