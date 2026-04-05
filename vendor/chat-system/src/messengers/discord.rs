//! Discord messenger — REST API + Gateway implementation.

use crate::{Message, Messenger};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chrono::DateTime;
use futures::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::{sync::Mutex, task::JoinHandle, time::Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

pub struct DiscordMessenger {
    name: String,
    token: String,
    api_base_url: String,
    gateway_url_override: Option<String>,
    client: Client,
    message_queue: Arc<Mutex<Vec<Message>>>,
    gateway_task: Option<JoinHandle<()>>,
    connected: bool,
}

impl DiscordMessenger {
    pub fn new(name: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            token: token.into(),
            api_base_url: "https://discord.com/api/v10".to_string(),
            gateway_url_override: None,
            client: Client::new(),
            message_queue: Arc::new(Mutex::new(Vec::new())),
            gateway_task: None,
            connected: false,
        }
    }

    pub fn with_api_base_url(mut self, url: impl Into<String>) -> Self {
        self.api_base_url = url.into();
        self
    }

    pub fn with_gateway_url(mut self, url: impl Into<String>) -> Self {
        self.gateway_url_override = Some(url.into());
        self
    }

    fn api_url(&self, path: impl AsRef<str>) -> String {
        format!("{}/{}", self.api_base_url.trim_end_matches('/'), path.as_ref().trim_start_matches('/'))
    }

    fn authorization_header(&self) -> String {
        format!("Bot {}", self.token)
    }

    async fn get_gateway_url(&self) -> Result<String> {
        if let Some(url) = &self.gateway_url_override {
            return Ok(url.clone());
        }

        let response = self
            .client
            .get(self.api_url("gateway/bot"))
            .header("Authorization", self.authorization_header())
            .send()
            .await
            .context("Failed to request Discord gateway URL")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Discord gateway lookup failed {}: {}", status, body);
        }

        let payload: Value = response.json().await.context("Invalid Discord gateway response")?;
        let gateway_url = payload["url"]
            .as_str()
            .ok_or_else(|| anyhow!("Discord gateway response missing url"))?;
        Ok(format!("{}?v=10&encoding=json", gateway_url))
    }

    async fn spawn_gateway_task(&mut self, gateway_url: String) -> Result<()> {
        let (stream, _) = connect_async(gateway_url)
            .await
            .context("Failed to connect to Discord gateway")?;
        let (mut writer, mut reader) = stream.split();

        let hello = reader
            .next()
            .await
            .ok_or_else(|| anyhow!("Discord gateway closed before HELLO"))?
            .context("Failed to read Discord HELLO")?;

        let hello_text = match hello {
            WsMessage::Text(text) => text.to_string(),
            other => anyhow::bail!("Unexpected Discord gateway HELLO frame: {other:?}"),
        };
        let hello_payload: Value = serde_json::from_str(&hello_text).context("Invalid Discord HELLO payload")?;
        let heartbeat_interval = hello_payload["d"]["heartbeat_interval"]
            .as_u64()
            .ok_or_else(|| anyhow!("Discord HELLO missing heartbeat_interval"))?;

        let identify = json!({
            "op": 2,
            "d": {
                "token": self.token,
                "intents": 513,
                "properties": {
                    "os": std::env::consts::OS,
                    "browser": "chat-system",
                    "device": "chat-system"
                }
            }
        });
        writer
            .send(WsMessage::Text(identify.to_string().into()))
            .await
            .context("Failed to identify with Discord gateway")?;

        let message_queue = self.message_queue.clone();
        let task = tokio::spawn(async move {
            let mut sequence = None::<Value>;
            let mut heartbeat = tokio::time::interval(Duration::from_millis(heartbeat_interval));
            heartbeat.tick().await;

            loop {
                tokio::select! {
                    _ = heartbeat.tick() => {
                        let payload = json!({ "op": 1, "d": sequence.clone().unwrap_or(Value::Null) });
                        if writer.send(WsMessage::Text(payload.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    maybe_message = reader.next() => {
                        let Some(message) = maybe_message else { break; };
                        let Ok(message) = message else { break; };

                        let text = match message {
                            WsMessage::Text(text) => text.to_string(),
                            WsMessage::Close(_) => break,
                            _ => continue,
                        };

                        let Ok(payload) = serde_json::from_str::<Value>(&text) else {
                            continue;
                        };

                        if let Some(seq_value) = payload.get("s").cloned() {
                            if !seq_value.is_null() {
                                sequence = Some(seq_value);
                            }
                        }

                        match payload["op"].as_i64() {
                            Some(0) => {
                                if payload["t"].as_str() == Some("MESSAGE_CREATE") {
                                    let data = &payload["d"];
                                    let timestamp = data["timestamp"]
                                        .as_str()
                                        .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
                                        .map(|ts| ts.timestamp())
                                        .unwrap_or_else(|| chrono::Utc::now().timestamp());

                                    let message = Message {
                                        id: data["id"].as_str().unwrap_or("").to_string(),
                                        sender: data["author"]["username"]
                                            .as_str()
                                            .unwrap_or("unknown")
                                            .to_string(),
                                        content: data["content"].as_str().unwrap_or("").to_string(),
                                        timestamp,
                                        channel: data["channel_id"].as_str().map(|s| s.to_string()),
                                        reply_to: data["message_reference"]["message_id"]
                                            .as_str()
                                            .map(|s| s.to_string()),
                                        media: None,
                                        is_direct: data.get("guild_id").is_none() || data["guild_id"].is_null(),
                                        reactions: None,
                                    };

                                    let mut queue = message_queue.lock().await;
                                    queue.push(message);
                                }
                            }
                            Some(1) => {
                                let payload = json!({ "op": 1, "d": sequence.clone().unwrap_or(Value::Null) });
                                if writer.send(WsMessage::Text(payload.to_string().into())).await.is_err() {
                                    break;
                                }
                            }
                            Some(7) | Some(9) => break,
                            _ => {}
                        }
                    }
                }
            }
        });

        self.gateway_task = Some(task);
        Ok(())
    }
}

#[async_trait]
impl Messenger for DiscordMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "discord"
    }

    async fn initialize(&mut self) -> Result<()> {
        let resp = self
            .client
            .get(self.api_url("users/@me"))
            .header("Authorization", self.authorization_header())
            .send()
            .await
            .context("Failed to verify Discord token")?;

        if !resp.status().is_success() {
            anyhow::bail!("Discord auth failed: {}", resp.status());
        }

        let gateway_url = self.get_gateway_url().await?;
        self.spawn_gateway_task(gateway_url).await?;

        self.connected = true;
        Ok(())
    }

    async fn send_message(&self, channel_id: &str, content: &str) -> Result<String> {
        let url = self.api_url(format!("channels/{channel_id}/messages"));
        let body = json!({ "content": content });

        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.authorization_header())
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            let data: Value = resp.json().await?;
            let id = data["id"].as_str().unwrap_or("").to_string();
            Ok(id)
        } else {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Discord send_message failed {}: {}", status, text);
        }
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        let mut queue = self.message_queue.lock().await;
        Ok(std::mem::take(&mut *queue))
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(task) = self.gateway_task.take() {
            task.abort();
        }
        self.connected = false;
        Ok(())
    }

    async fn set_typing(&self, channel: &str, typing: bool) -> Result<()> {
        if !typing {
            return Ok(());
        }

        let response = self
            .client
            .post(self.api_url(format!("channels/{channel}/typing")))
            .header("Authorization", self.authorization_header())
            .send()
            .await
            .context("Failed to send Discord typing indicator")?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Discord typing failed {}: {}", status, body);
        }
    }
}
