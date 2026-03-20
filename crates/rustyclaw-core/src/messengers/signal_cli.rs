//! Signal messenger using signal-cli.
//! 
//! This messenger implementation uses the signal-cli command-line tool to interact
//! with Signal Private Messenger. The signal-cli tool must be installed and configured
//! separately before using this messenger.
//!
//! # Prerequisites
//!
//! 1. Install signal-cli from https://github.com/AsamK/signal-cli
//! 2. Register your phone number: `signal-cli -u +1234567890 register`
//! 3. Verify with code: `signal-cli -u +1234567890 verify CODE`
//!
//! # Configuration
//!
//! ```toml
//! [messengers.signal]
//! phone_number = "+1234567890"
//! signal_cli_path = "/usr/local/bin/signal-cli"  # Optional, defaults to "signal-cli"
//! ```

use super::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::process::Stdio;
use tokio::process::Command;
use chrono::Utc;

/// Signal messenger using signal-cli external process
pub struct SignalCliMessenger {
    name: String,
    phone_number: String,
    signal_cli_path: String,
    connected: bool,
    last_sync_timestamp: i64,
}

impl SignalCliMessenger {
    /// Create a new Signal CLI messenger
    pub fn new(name: String, phone_number: String) -> Self {
        Self {
            name,
            phone_number,
            signal_cli_path: "signal-cli".to_string(),
            connected: false,
            last_sync_timestamp: 0,
        }
    }

    /// Create a new Signal CLI messenger with custom signal-cli path
    pub fn new_with_path(name: String, phone_number: String, signal_cli_path: String) -> Self {
        Self {
            name,
            phone_number,
            signal_cli_path,
            connected: false,
            last_sync_timestamp: 0,
        }
    }

    /// Execute a signal-cli command and return the output
    async fn execute_signal_cli(&self, args: &[&str]) -> Result<String> {
        let mut cmd = Command::new(&self.signal_cli_path);
        cmd.args(["-u", &self.phone_number])
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = cmd
            .output()
            .await
            .with_context(|| format!("Failed to execute signal-cli at '{}'", self.signal_cli_path))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("signal-cli command failed: {}", stderr);
        }

        let stdout = String::from_utf8(output.stdout)
            .context("signal-cli output is not valid UTF-8")?;
        
        Ok(stdout)
    }

    /// Execute a signal-cli command that expects JSON output
    async fn execute_signal_cli_json(&self, args: &[&str]) -> Result<serde_json::Value> {
        let output = self.execute_signal_cli(args).await?;
        
        if output.trim().is_empty() {
            return Ok(serde_json::json!([]));
        }

        // Handle both single JSON objects and newline-separated JSON objects
        let mut results = Vec::new();
        for line in output.lines() {
            let line = line.trim();
            if !line.is_empty() {
                match serde_json::from_str::<serde_json::Value>(line) {
                    Ok(json) => results.push(json),
                    Err(e) => {
                        tracing::warn!("Failed to parse signal-cli JSON output: {} (line: {})", e, line);
                    }
                }
            }
        }

        if results.is_empty() {
            Ok(serde_json::json!([]))
        } else if results.len() == 1 {
            Ok(results.into_iter().next().unwrap())
        } else {
            Ok(serde_json::Value::Array(results))
        }
    }

    /// Check if signal-cli is available and properly configured
    async fn check_signal_cli(&self) -> Result<()> {
        // First check if signal-cli is available
        let version_output = Command::new(&self.signal_cli_path)
            .arg("--version")
            .output()
            .await
            .with_context(|| {
                format!(
                    "signal-cli not found at '{}'. Please install signal-cli from https://github.com/AsamK/signal-cli", 
                    self.signal_cli_path
                )
            })?;

        if !version_output.status.success() {
            anyhow::bail!("signal-cli is not working properly");
        }

        // Then check if the account is registered
        match self.execute_signal_cli(&["whoami"]).await {
            Ok(_) => Ok(()),
            Err(_) => {
                anyhow::bail!(
                    "Signal account {} is not registered. Please run:\n  signal-cli -u {} register\n  signal-cli -u {} verify CODE",
                    self.phone_number, self.phone_number, self.phone_number
                )
            }
        }
    }

    /// Normalize phone number to Signal format (E.164)
    fn normalize_phone_number(phone: &str) -> String {
        let cleaned = phone.chars()
            .filter(|c| c.is_ascii_digit() || *c == '+')
            .collect::<String>();
        
        if cleaned.starts_with('+') {
            cleaned
        } else if cleaned.len() == 10 {
            format!("+1{}", cleaned) // Assume US number
        } else {
            format!("+{}", cleaned.trim_start_matches('1'))
        }
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
        self.check_signal_cli().await?;
        
        // Sync messages to make sure we can communicate
        match self.execute_signal_cli(&["receive", "--timeout", "1"]).await {
            Ok(_) => {
                self.connected = true;
                self.last_sync_timestamp = Utc::now().timestamp();
                Ok(())
            }
            Err(e) => {
                anyhow::bail!("Failed to initialize Signal messenger: {}", e);
            }
        }
    }

    async fn send_message(&self, recipient: &str, content: &str) -> Result<String> {
        if !self.connected {
            anyhow::bail!("Signal messenger is not connected");
        }

        let normalized_recipient = Self::normalize_phone_number(recipient);
        
        self.execute_signal_cli(&[
            "send",
            "-m", content,
            &normalized_recipient,
        ]).await?;

        // signal-cli doesn't return message IDs, so we generate one based on timestamp
        let message_id = format!("signal_{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        Ok(message_id)
    }

    async fn send_message_with_options(&self, opts: SendOptions<'_>) -> Result<String> {
        if !self.connected {
            anyhow::bail!("Signal messenger is not connected");
        }

        let normalized_recipient = Self::normalize_phone_number(opts.recipient);
        let mut args = vec!["send", "-m", opts.content];
        
        // Note: signal-cli doesn't support reply_to or silent options directly
        // We could prepend reply context to the message content if needed
        if let Some(reply_to) = opts.reply_to {
            let content_with_reply = format!("↳ Reply to {}\n\n{}", reply_to, opts.content);
            args[2] = &content_with_reply;
        }
        
        args.push(&normalized_recipient);

        // Handle media attachment
        if let Some(media_path) = opts.media {
            args.insert(1, "-a");
            args.insert(2, media_path);
        }

        self.execute_signal_cli(&args).await?;

        let message_id = format!("signal_{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        Ok(message_id)
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        if !self.connected {
            return Ok(Vec::new());
        }

        // Use --json flag for structured output
        let json_output = self.execute_signal_cli_json(&[
            "receive",
            "--timeout", "1",
            "--json",
        ]).await?;

        let mut messages = Vec::new();
        let current_timestamp = Utc::now().timestamp();

        // Handle both array and single message responses
        let message_array = if json_output.is_array() {
            json_output.as_array().unwrap().clone()
        } else if json_output.is_object() && json_output.get("envelope").is_some() {
            vec![json_output]
        } else {
            Vec::new()
        };

        for msg_data in message_array {
            if let Some(envelope) = msg_data.get("envelope") {
                if let Some(data_message) = envelope.get("dataMessage") {
                    if let Some(message_text) = data_message.get("message").and_then(|m| m.as_str()) {
                        let sender = envelope
                            .get("source")
                            .and_then(|s| s.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        
                        let timestamp = envelope
                            .get("timestamp")
                            .and_then(|t| t.as_i64())
                            .map(|t| t / 1000) // Convert from milliseconds to seconds
                            .unwrap_or(current_timestamp);
                        
                        // Only include messages newer than our last sync
                        if timestamp > self.last_sync_timestamp {
                            let message_id = format!("signal_{}_{}", timestamp, sender);
                            
                            // Extract group info if present
                            let channel = envelope
                                .get("dataMessage")
                                .and_then(|dm| dm.get("groupInfo"))
                                .and_then(|gi| gi.get("groupId"))
                                .and_then(|gid| gid.as_str())
                                .map(|s| s.to_string());

                            messages.push(Message {
                                id: message_id,
                                sender,
                                content: message_text.to_string(),
                                timestamp,
                                channel,
                                reply_to: None, // signal-cli JSON doesn't include reply info easily
                                media: None,    // TODO: Handle attachments
                            });
                        }
                    }
                }
            }
        }

        Ok(messages)
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    async fn disconnect(&mut self) -> Result<()> {
        // signal-cli doesn't have an explicit disconnect, just mark as disconnected
        self.connected = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_phone_number() {
        assert_eq!(SignalCliMessenger::normalize_phone_number("+1234567890"), "+1234567890");
        assert_eq!(SignalCliMessenger::normalize_phone_number("1234567890"), "+11234567890");
        assert_eq!(SignalCliMessenger::normalize_phone_number("234567890"), "+1234567890");
        assert_eq!(SignalCliMessenger::normalize_phone_number("+49123456789"), "+49123456789");
        assert_eq!(SignalCliMessenger::normalize_phone_number("(555) 123-4567"), "+15551234567");
    }

    #[test]
    fn test_messenger_creation() {
        let messenger = SignalCliMessenger::new(
            "test_signal".to_string(),
            "+1234567890".to_string(),
        );
        
        assert_eq!(messenger.name(), "test_signal");
        assert_eq!(messenger.messenger_type(), "signal");
        assert!(!messenger.is_connected());
    }
}