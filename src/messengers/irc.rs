//! IRC messenger with basic TCP support.
//!
//! This integration supports simple outbound messaging and best-effort polling
//! for inbound `PRIVMSG` lines. It uses plain TCP and does not currently
//! implement SASL or TLS.

use crate::messengers::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct IrcConfig {
    pub server: String,
    pub port: u16,
    pub nickname: String,
    pub username: String,
    pub realname: String,
    pub password: Option<String>,
    pub channel: Option<String>,
}

impl Default for IrcConfig {
    fn default() -> Self {
        Self {
            server: "irc.libera.chat".to_string(),
            port: 6667,
            nickname: "rustyclaw".to_string(),
            username: "rustyclaw".to_string(),
            realname: "RustyClaw".to_string(),
            password: None,
            channel: None,
        }
    }
}

pub struct IrcMessenger {
    name: String,
    config: IrcConfig,
    stream: Arc<Mutex<Option<TcpStream>>>,
    pending: Arc<Mutex<String>>,
    connected: AtomicBool,
}

impl IrcMessenger {
    pub fn new(name: String, config: IrcConfig) -> Self {
        Self {
            name,
            config,
            stream: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(String::new())),
            connected: AtomicBool::new(false),
        }
    }

    fn address(&self) -> String {
        format!("{}:{}", self.config.server, self.config.port)
    }

    fn channel_for_send(&self, recipient: &str) -> Result<String> {
        if !recipient.trim().is_empty() {
            return Ok(recipient.to_string());
        }

        self.config
            .channel
            .clone()
            .context("IRC requires recipient channel/user or default channel")
    }

    async fn send_raw_line(&self, line: &str) -> Result<()> {
        let mut guard = self.stream.lock().await;
        let stream = guard.as_mut().context("IRC stream is not connected")?;

        stream
            .write_all(line.as_bytes())
            .await
            .context("Failed to write IRC line")?;
        stream
            .write_all(b"\r\n")
            .await
            .context("Failed to write IRC line terminator")?;
        stream.flush().await.context("Failed to flush IRC line")?;

        Ok(())
    }

    fn parse_privmsg(line: &str) -> Option<(String, String, String)> {
        // Typical format:
        // :nick!user@host PRIVMSG #channel :hello world
        if !line.contains(" PRIVMSG ") || !line.starts_with(':') {
            return None;
        }

        let mut parts = line.splitn(4, ' ');
        let prefix = parts.next()?;
        let command = parts.next()?;
        let target = parts.next()?;
        let body = parts.next()?;

        if command != "PRIVMSG" {
            return None;
        }

        let sender = prefix
            .trim_start_matches(':')
            .split('!')
            .next()
            .unwrap_or("")
            .to_string();
        let content = body.strip_prefix(':').unwrap_or(body).to_string();

        if sender.is_empty() || content.is_empty() {
            return None;
        }

        Some((sender, target.to_string(), content))
    }
}

#[async_trait]
impl Messenger for IrcMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "irc"
    }

    async fn initialize(&mut self) -> Result<()> {
        let mut stream = TcpStream::connect(self.address())
            .await
            .with_context(|| format!("Failed to connect to IRC server {}", self.address()))?;

        if let Some(password) = &self.config.password {
            stream
                .write_all(format!("PASS {}\r\n", password).as_bytes())
                .await
                .context("Failed to send IRC PASS")?;
        }

        stream
            .write_all(format!("NICK {}\r\n", self.config.nickname).as_bytes())
            .await
            .context("Failed to send IRC NICK")?;

        stream
            .write_all(
                format!(
                    "USER {} 0 * :{}\r\n",
                    self.config.username, self.config.realname
                )
                .as_bytes(),
            )
            .await
            .context("Failed to send IRC USER")?;

        if let Some(channel) = &self.config.channel {
            stream
                .write_all(format!("JOIN {}\r\n", channel).as_bytes())
                .await
                .context("Failed to send IRC JOIN")?;
        }

        stream.flush().await.context("Failed to flush IRC auth")?;

        *self.stream.lock().await = Some(stream);
        self.connected.store(true, Ordering::SeqCst);
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
        let target = self.channel_for_send(opts.recipient)?;
        let body = opts.content.replace('\r', " ").replace('\n', " ");
        self.send_raw_line(&format!("PRIVMSG {} :{}", target, body))
            .await?;

        Ok(format!("irc-{}", chrono::Utc::now().timestamp_millis()))
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        if !self.connected.load(Ordering::SeqCst) {
            return Ok(Vec::new());
        }

        let mut raw = String::new();

        {
            let mut guard = self.stream.lock().await;
            let Some(stream) = guard.as_mut() else {
                self.connected.store(false, Ordering::SeqCst);
                return Ok(Vec::new());
            };

            let mut chunk = [0_u8; 4096];
            loop {
                let readable =
                    tokio::time::timeout(Duration::from_millis(15), stream.readable()).await;
                match readable {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => return Err(e).context("IRC stream became unreadable"),
                    Err(_) => break,
                }

                match stream.try_read(&mut chunk) {
                    Ok(0) => {
                        self.connected.store(false, Ordering::SeqCst);
                        *guard = None;
                        break;
                    }
                    Ok(n) => raw.push_str(&String::from_utf8_lossy(&chunk[..n])),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e).context("Failed to read IRC stream"),
                }
            }
        }

        if raw.is_empty() {
            return Ok(Vec::new());
        }

        let lines = {
            let mut pending = self.pending.lock().await;
            pending.push_str(&raw);
            let mut lines = Vec::new();

            while let Some(idx) = pending.find('\n') {
                let mut line = pending.drain(..=idx).collect::<String>();
                while line.ends_with('\n') || line.ends_with('\r') {
                    line.pop();
                }
                if !line.is_empty() {
                    lines.push(line);
                }
            }

            lines
        };

        let mut messages = Vec::new();

        for line in lines {
            if let Some(payload) = line.strip_prefix("PING ") {
                let _ = self.send_raw_line(&format!("PONG {}", payload)).await;
                continue;
            }

            if let Some((sender, channel, content)) = Self::parse_privmsg(&line) {
                messages.push(Message {
                    id: format!("irc-{}", chrono::Utc::now().timestamp_millis()),
                    sender,
                    content,
                    timestamp: chrono::Utc::now().timestamp(),
                    channel: Some(channel),
                    reply_to: None,
                    media: None,
                });
            }
        }

        Ok(messages)
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn disconnect(&mut self) -> Result<()> {
        if self.connected.load(Ordering::SeqCst) {
            let _ = self.send_raw_line("QUIT :RustyClaw disconnect").await;
        }

        *self.stream.lock().await = None;
        self.connected.store(false, Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_irc_type() {
        let m = IrcMessenger::new("irc-main".to_string(), IrcConfig::default());
        assert_eq!(m.messenger_type(), "irc");
    }

    #[test]
    fn test_parse_privmsg() {
        let line = ":alice!u@h PRIVMSG #rust :hello";
        let parsed = IrcMessenger::parse_privmsg(line).unwrap();
        assert_eq!(parsed.0, "alice");
        assert_eq!(parsed.1, "#rust");
        assert_eq!(parsed.2, "hello");
    }
}
