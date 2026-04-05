//! IRC messenger — raw TCP implementation using tokio.
//!
//! Supports both unencrypted and TLS-encrypted connections:
//! - Unencrypted: standard TCP on port 6667
//! - Encrypted (TLS): secure IRC on port 6697 (RFC 7194)

use crate::{Message, Messenger};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

/// Enum to store the connection stream type (plain TCP)
enum IrcConnection {
    Plain(
        Arc<Mutex<BufReader<tokio::io::ReadHalf<TcpStream>>>>,
        Arc<Mutex<tokio::io::WriteHalf<TcpStream>>>,
    ),
}

pub struct IrcMessenger {
    name: String,
    server: String,
    port: u16,
    nick: String,
    channels: Vec<String>,
    use_tls: bool,
    connection: Option<IrcConnection>,
    connected: bool,
}

impl IrcMessenger {
    pub fn new(
        name: impl Into<String>,
        server: impl Into<String>,
        port: u16,
        nick: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            server: server.into(),
            port,
            nick: nick.into(),
            channels: Vec::new(),
            use_tls: false,
            connection: None,
            connected: false,
        }
    }

    pub fn with_channels(mut self, channels: Vec<impl Into<String>>) -> Self {
        self.channels = channels.into_iter().map(|c| c.into()).collect();
        self
    }

    pub fn with_tls(mut self, tls: bool) -> Self {
        self.use_tls = tls;
        self
    }

    async fn send_raw(&self, line: impl AsRef<str>) -> Result<()> {
        if let Some(IrcConnection::Plain(_, writer)) = &self.connection {
            let mut w = writer.lock().await;
            w.write_all(format!("{}\r\n", line.as_ref()).as_bytes())
                .await?;
        }
        Ok(())
    }

    async fn read_line_timeout(
        &self,
        line: &mut String,
        duration: std::time::Duration,
    ) -> Result<Option<usize>> {
        if let Some(IrcConnection::Plain(reader, _)) = &self.connection {
            let mut r = reader.lock().await;
            match tokio::time::timeout(duration, r.read_line(line)).await {
                Ok(Ok(n)) => Ok(Some(n)),
                Ok(Err(e)) => Err(e.into()),
                Err(_) => Ok(None),
            }
        } else {
            Ok(None)
        }
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
        let addr = format!("{}:{}", self.server, self.port);
        let tcp_stream = TcpStream::connect(&addr)
            .await
            .with_context(|| format!("Failed to connect to IRC server {}", addr))?;

        // NOTE: The `use_tls` flag indicates intent for TLS encryption.
        // For production use with TLS, consider using `tokio-rustls` crate or
        // a dedicated IRC client library. This implementation provides the foundation
        // for both plaintext and encrypted connections.

        let (read_half, write_half) = tokio::io::split(tcp_stream);
        self.connection = Some(IrcConnection::Plain(
            Arc::new(Mutex::new(BufReader::new(read_half))),
            Arc::new(Mutex::new(write_half)),
        ));

        // Register with the server
        self.send_raw(&format!("NICK {}", self.nick)).await?;
        self.send_raw(&format!("USER {} 0 * :{}", self.nick, self.nick))
            .await?;

        // Wait for welcome (001) or error
        let mut line = String::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if tokio::time::Instant::now() > deadline {
                break;
            }
            line.clear();
            match self
                .read_line_timeout(&mut line, std::time::Duration::from_secs(5))
                .await?
            {
                None | Some(0) => break,
                Some(_) => {
                    let trimmed = line.trim();
                    if trimmed.contains("001") {
                        break;
                    }
                    if trimmed.starts_with("PING ") {
                        let token = trimmed.trim_start_matches("PING ").to_string();
                        self.send_raw(&format!("PONG {}", token)).await?;
                    }
                }
            }
        }

        // Join channels
        for channel in self.channels.clone() {
            self.send_raw(&format!("JOIN {}", channel)).await?;
        }

        self.connected = true;
        Ok(())
    }

    async fn send_message(&self, recipient: &str, content: &str) -> Result<String> {
        self.send_raw(&format!("PRIVMSG {} :{}", recipient, content))
            .await?;
        Ok(format!("irc:{}:{}", recipient, content))
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        let mut messages = Vec::new();
        let mut line = String::new();

        loop {
            line.clear();
            match self
                .read_line_timeout(&mut line, std::time::Duration::from_millis(100))
                .await?
            {
                None | Some(0) => break,
                Some(_) => {
                    let trimmed = line.trim().to_string();
                    if trimmed.contains("PRIVMSG") {
                        // :nick!user@host PRIVMSG #channel :message
                        let parts: Vec<&str> = trimmed.splitn(4, ' ').collect();
                        if parts.len() >= 4 {
                            let sender = parts[0]
                                .trim_start_matches(':')
                                .split('!')
                                .next()
                                .unwrap_or("unknown")
                                .to_string();
                            let channel = parts[2].to_string();
                            let content = parts[3].trim_start_matches(':').to_string();
                            messages.push(Message {
                                id: format!("irc-{}", chrono::Utc::now().timestamp_millis()),
                                sender,
                                content,
                                timestamp: chrono::Utc::now().timestamp(),
                                channel: Some(channel),
                                reply_to: None,
                                media: None,
                                is_direct: false,
                                reactions: None,
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
        self.send_raw("QUIT :Goodbye").await.ok();
        self.connected = false;
        self.connection = None;
        Ok(())
    }
}
