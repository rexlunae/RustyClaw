//! Signal messenger — signal-cli subprocess wrapper.

use crate::{Message, Messenger};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use tokio::process::Command;

pub struct SignalCliMessenger {
    name: String,
    phone_number: String,
    signal_cli_path: String,
    connected: bool,
}

impl SignalCliMessenger {
    pub fn new(name: impl Into<String>, phone_number: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            phone_number: phone_number.into(),
            signal_cli_path: "signal-cli".to_string(),
            connected: false,
        }
    }

    pub fn with_cli_path(mut self, path: impl Into<String>) -> Self {
        self.signal_cli_path = path.into();
        self
    }

    async fn run_signal_cli(&self, args: &[&str], operation: &str) -> Result<std::process::Output> {
        Command::new(&self.signal_cli_path)
            .args(args)
            .output()
            .await
            .with_context(|| format!("Failed to spawn signal-cli for {operation}"))
    }

    fn parse_receive_output(&self, stdout: &[u8]) -> Result<Vec<Message>> {
        let text = String::from_utf8_lossy(stdout);
        let mut messages = Vec::new();

        for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
            let value: Value = serde_json::from_str(line)
                .with_context(|| format!("Invalid signal-cli receive JSON: {line}"))?;
            if let Some(message) = Self::parse_signal_message(&value) {
                messages.push(message);
            }
        }

        Ok(messages)
    }

    fn parse_signal_message(value: &Value) -> Option<Message> {
        let envelope = value.get("envelope").unwrap_or(value);
        let data_message = envelope
            .get("dataMessage")
            .or_else(|| envelope.get("syncMessage")?.get("sentMessage"))?;

        let content = data_message.get("message")?.as_str()?.to_string();
        let raw_timestamp = envelope
            .get("timestamp")
            .and_then(Value::as_i64)
            .or_else(|| data_message.get("timestamp").and_then(Value::as_i64))
            .unwrap_or_default();
        let timestamp = if raw_timestamp > 10_000_000_000 {
            raw_timestamp / 1_000
        } else {
            raw_timestamp
        };

        let channel = data_message
            .get("groupInfo")
            .and_then(|group| group.get("groupId"))
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let id = envelope
            .get("timestamp")
            .or_else(|| data_message.get("timestamp"))
            .and_then(Value::as_i64)
            .map(|value| value.to_string())
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis().to_string());
        let sender = envelope
            .get("sourceName")
            .and_then(Value::as_str)
            .or_else(|| envelope.get("sourceNumber").and_then(Value::as_str))
            .or_else(|| envelope.get("source").and_then(Value::as_str))
            .unwrap_or("unknown")
            .to_string();
        let reply_to = data_message
            .get("quote")
            .and_then(|quote| quote.get("id"))
            .and_then(Value::as_i64)
            .map(|value| value.to_string());

        Some(Message {
            id,
            sender,
            content,
            timestamp,
            channel: channel.clone(),
            reply_to,
            media: None,
            is_direct: channel.is_none(),
            reactions: None,
        })
    }
}

#[async_trait]
impl Messenger for SignalCliMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "signal"
    }

    async fn initialize(&mut self) -> Result<()> {
        let output = self.run_signal_cli(&["--version"], "initialize").await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("signal-cli initialization failed: {}", stderr.trim());
        }

        self.connected = true;
        Ok(())
    }

    async fn send_message(&self, recipient: &str, content: &str) -> Result<String> {
        let output = self
            .run_signal_cli(
                &["-u", &self.phone_number, "send", "-m", content, recipient],
                "send",
            )
            .await?;

        if output.status.success() {
            Ok(format!("signal:{}", chrono::Utc::now().timestamp_millis()))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("signal-cli send failed: {}", stderr);
        }
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        if !self.connected {
            return Ok(Vec::new());
        }

        let output = self
            .run_signal_cli(
                &[
                    "-u",
                    &self.phone_number,
                    "receive",
                    "--output",
                    "json",
                    "--timeout",
                    "1",
                ],
                "receive",
            )
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("signal-cli receive failed: {}", stderr.trim());
        }

        self.parse_receive_output(&output.stdout)
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.connected = false;
        Ok(())
    }
}
