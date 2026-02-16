//! Telegram messenger using Bot API.

use super::{Message, Messenger, SendOptions};
use anyhow::Result;
use async_trait::async_trait;

/// Telegram messenger using bot API
pub struct TelegramMessenger {
    name: String,
    bot_token: String,
    connected: bool,
    http: reqwest::Client,
    last_update_id: i64,
}

impl TelegramMessenger {
    pub fn new(name: String, bot_token: String) -> Self {
        Self {
            name,
            bot_token,
            connected: false,
            http: reqwest::Client::new(),
            last_update_id: 0,
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{}", self.bot_token, method)
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
        // Verify bot token with getMe
        let resp = self.http.get(self.api_url("getMe")).send().await?;

        if resp.status().is_success() {
            let data: serde_json::Value = resp.json().await?;
            if data["ok"].as_bool() == Some(true) {
                self.connected = true;
                return Ok(());
            }
        }
        anyhow::bail!("Telegram auth failed")
    }

    async fn send_message(&self, chat_id: &str, content: &str) -> Result<String> {
        let resp = self
            .http
            .post(self.api_url("sendMessage"))
            .json(&serde_json::json!({
                "chat_id": chat_id,
                "text": content,
                "parse_mode": "Markdown"
            }))
            .send()
            .await?;

        if resp.status().is_success() {
            let data: serde_json::Value = resp.json().await?;
            if data["ok"].as_bool() == Some(true) {
                return Ok(data["result"]["message_id"].to_string());
            }
        }
        anyhow::bail!("Telegram send failed")
    }

    async fn send_message_with_options(&self, opts: SendOptions<'_>) -> Result<String> {
        let mut payload = serde_json::json!({
            "chat_id": opts.recipient,
            "text": opts.content,
            "parse_mode": "Markdown"
        });

        if opts.silent {
            payload["disable_notification"] = serde_json::json!(true);
        }

        if let Some(reply_to) = opts.reply_to {
            if let Ok(msg_id) = reply_to.parse::<i64>() {
                payload["reply_to_message_id"] = serde_json::json!(msg_id);
            }
        }

        let resp = self
            .http
            .post(self.api_url("sendMessage"))
            .json(&payload)
            .send()
            .await?;

        if resp.status().is_success() {
            let data: serde_json::Value = resp.json().await?;
            if data["ok"].as_bool() == Some(true) {
                return Ok(data["result"]["message_id"].to_string());
            }
        }
        anyhow::bail!("Telegram send failed")
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        let resp = self
            .http
            .post(self.api_url("getUpdates"))
            .json(&serde_json::json!({
                "offset": self.last_update_id + 1,
                "timeout": 0,
                "allowed_updates": ["message"]
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(Vec::new());
        }

        let data: serde_json::Value = resp.json().await?;
        if data["ok"].as_bool() != Some(true) {
            return Ok(Vec::new());
        }

        let updates = data["result"].as_array();
        let Some(updates) = updates else {
            return Ok(Vec::new());
        };

        let mut messages = Vec::new();
        for update in updates {
            let _update_id = update["update_id"].as_i64().unwrap_or(0);
            
            if let Some(msg) = update.get("message") {
                let id = msg["message_id"].to_string();
                let sender = msg["from"]["id"].to_string();
                let content = msg["text"].as_str().unwrap_or("").to_string();
                let timestamp = msg["date"].as_i64().unwrap_or(0);
                let channel = msg["chat"]["id"].to_string();

                messages.push(Message {
                    id,
                    sender,
                    content,
                    timestamp,
                    channel: Some(channel),
                    reply_to: msg["reply_to_message"]["message_id"]
                        .as_i64()
                        .map(|id| id.to_string()),
                    media: None,
                });
            }
        }

        Ok(messages)
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.connected = false;
        Ok(())
    }

    async fn set_typing(&self, chat_id: &str, typing: bool) -> Result<()> {
        if !typing {
            // Telegram doesn't have an explicit "stop typing" action
            // The typing indicator auto-expires after 5 seconds
            return Ok(());
        }

        let _ = self
            .http
            .post(self.api_url("sendChatAction"))
            .json(&serde_json::json!({
                "chat_id": chat_id,
                "action": "typing"
            }))
            .send()
            .await;

        // Ignore errors - typing indicator is best-effort
        Ok(())
    }
}
