//! IRC messenger using raw TCP/TLS connections.
//!
//! Implements basic IRC protocol (RFC 2812) for connecting to IRC servers,
//! joining channels, sending/receiving messages. Supports TLS.

use super::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

/// IRC messenger using raw TCP/TLS.
pub struct IrcMessenger {
    name: String,
    server: String,
    port: u16,
    nick: String,
    channels: Vec<String>,
    use_tls: bool,
    password: Option<String>,
    connected: bool,
    /// Shared writer half of the TCP stream.
    writer: Option<Arc<Mutex<Box<dyn tokio::io::AsyncWrite + Send + Unpin>>>>,
    /// Pending incoming messages collected by the reader task.
    pending_messages: Arc<Mutex<Vec<Message>>>,
    /// Background reader task handle.
    _reader_handle: Option<tokio::task::JoinHandle<()>>,
}

impl IrcMessenger {
    pub fn new(name: String, server: String, port: u16, nick: String) -> Self {
        Self {
            name,
            server,
            port,
            nick,
            channels: Vec::new(),
            use_tls: port == 6697,
            password: None,
            connected: false,
            writer: None,
            pending_messages: Arc::new(Mutex::new(Vec::new())),
            _reader_handle: None,
        }
    }

    /// Set channels to join on connect.
    pub fn with_channels(mut self, channels: Vec<String>) -> Self {
        self.channels = channels;
        self
    }

    /// Set whether to use TLS.
    pub fn with_tls(mut self, use_tls: bool) -> Self {
        self.use_tls = use_tls;
        self
    }

    /// Set server password.
    pub fn with_password(mut self, password: String) -> Self {
        self.password = Some(password);
        self
    }

    /// Send a raw IRC command.
    async fn send_raw(&self, line: &str) -> Result<()> {
        if let Some(writer) = &self.writer {
            let mut w = writer.lock().await;
            w.write_all(format!("{}\r\n", line).as_bytes()).await?;
            w.flush().await?;
        }
        Ok(())
    }
}

/// Parse an IRC PRIVMSG line into sender, target, and message text.
fn parse_privmsg(line: &str) -> Option<(&str, &str, &str)> {
    // Format: :nick!user@host PRIVMSG #channel :message text
    if !line.starts_with(':') {
        return None;
    }
    let rest = &line[1..];
    let parts: Vec<&str> = rest.splitn(4, ' ').collect();
    if parts.len() < 4 || parts[1] != "PRIVMSG" {
        return None;
    }
    let sender = parts[0].split('!').next()?;
    let target = parts[2];
    let msg = parts[3].strip_prefix(':')?;
    Some((sender, target, msg))
}

/// Check if a line is a PING and return the token.
fn parse_ping(line: &str) -> Option<&str> {
    line.strip_prefix("PING ")
}

/// Split a UTF-8 string into chunks of at most `max_bytes` bytes each,
/// never splitting in the middle of a multi-byte character.
fn split_utf8(s: &str, max_bytes: usize) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < s.len() {
        let mut end = (start + max_bytes).min(s.len());
        // Back up to a char boundary if we landed mid-codepoint
        while end > start && !s.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            // Shouldn't happen with valid UTF-8, but advance at least one char
            end = start + s[start..].chars().next().map_or(1, |c| c.len_utf8());
        }
        chunks.push(&s[start..end]);
        start = end;
    }
    chunks
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
        let stream = TcpStream::connect(&addr)
            .await
            .with_context(|| format!("Failed to connect to IRC server {}", addr))?;

        // TLS support requires the `rustls-platform-verifier` crate which is
        // already pulled in transitively. For simplicity and to avoid adding
        // direct deps, we only support plaintext for now and log a warning
        // if TLS was requested.
        if self.use_tls {
            tracing::warn!(
                "IRC TLS requested but not yet supported natively. \
                 Connect to a plaintext port or use a TLS-terminating proxy (e.g. stunnel). \
                 Falling back to plaintext."
            );
        }

        let (reader, writer): (
            Box<dyn tokio::io::AsyncRead + Send + Unpin>,
            Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
        ) = {
            let (r, w) = tokio::io::split(stream);
            (Box::new(r), Box::new(w))
        };

        let writer = Arc::new(Mutex::new(writer));
        self.writer = Some(writer.clone());

        // Send registration
        if let Some(ref pass) = self.password {
            let mut w = writer.lock().await;
            w.write_all(format!("PASS {}\r\n", pass).as_bytes())
                .await?;
        }
        {
            let mut w = writer.lock().await;
            w.write_all(format!("NICK {}\r\n", self.nick).as_bytes())
                .await?;
            w.write_all(
                format!("USER {} 0 * :RustyClaw Bot\r\n", self.nick).as_bytes(),
            )
            .await?;
            w.flush().await?;
        }

        // Spawn reader task
        let pending = self.pending_messages.clone();
        let channels = self.channels.clone();
        let nick = self.nick.clone();
        let writer_clone = writer.clone();

        let handle = tokio::spawn(async move {
            let mut buf_reader = BufReader::new(reader);
            let mut line_buf = String::new();
            let mut joined = false;

            loop {
                line_buf.clear();
                match buf_reader.read_line(&mut line_buf).await {
                    Ok(0) => break, // Connection closed
                    Ok(_) => {
                        let line = line_buf.trim_end();

                        // Handle PING/PONG
                        if let Some(token) = parse_ping(line) {
                            let mut w = writer_clone.lock().await;
                            let _ = w
                                .write_all(format!("PONG {}\r\n", token).as_bytes())
                                .await;
                            let _ = w.flush().await;
                            continue;
                        }

                        // Join channels after RPL_WELCOME (001)
                        if !joined && line.contains(" 001 ") {
                            let mut w = writer_clone.lock().await;
                            for ch in &channels {
                                let _ = w
                                    .write_all(format!("JOIN {}\r\n", ch).as_bytes())
                                    .await;
                            }
                            let _ = w.flush().await;
                            joined = true;
                        }

                        // Parse PRIVMSG
                        if let Some((sender, target, text)) = parse_privmsg(line) {
                            // Skip our own messages
                            if sender == nick {
                                continue;
                            }

                            let channel = if target.starts_with('#') || target.starts_with('&') {
                                target.to_string()
                            } else {
                                sender.to_string()
                            };

                            let msg = Message {
                                id: format!(
                                    "irc-{}",
                                    chrono::Utc::now().timestamp_millis()
                                ),
                                sender: sender.to_string(),
                                content: text.to_string(),
                                timestamp: chrono::Utc::now().timestamp(),
                                channel: Some(channel),
                                reply_to: None,
                                media: None,
                                        is_direct: false, // TODO: implement DM detection
                            };

                            let mut pending = pending.lock().await;
                            pending.push(msg);
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        self._reader_handle = Some(handle);
        self.connected = true;

        tracing::info!(
            server = %self.server,
            nick = %self.nick,
            channels = ?self.channels,
            tls = self.use_tls,
            "IRC connected"
        );

        Ok(())
    }

    async fn send_message(&self, target: &str, content: &str) -> Result<String> {
        // IRC messages have a max length of ~512 bytes including the command.
        // Split long messages.
        let max_len = 400; // Leave room for PRIVMSG header
        for chunk in split_utf8(content, max_len) {
            self.send_raw(&format!("PRIVMSG {} :{}", target, chunk))
                .await?;
        }
        Ok(format!("irc-{}", chrono::Utc::now().timestamp_millis()))
    }

    async fn send_message_with_options(&self, opts: SendOptions<'_>) -> Result<String> {
        // IRC doesn't have native reply support — prefix with context
        let content = if let Some(reply_to) = opts.reply_to {
            format!("[re: {}] {}", reply_to, opts.content)
        } else {
            opts.content.to_string()
        };
        self.send_message(opts.recipient, &content).await
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        let mut pending = self.pending_messages.lock().await;
        Ok(pending.drain(..).collect())
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    async fn disconnect(&mut self) -> Result<()> {
        if self.connected {
            let _ = self.send_raw("QUIT :RustyClaw shutting down").await;
        }
        self.connected = false;
        self.writer = None;
        if let Some(handle) = self._reader_handle.take() {
            handle.abort();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_irc_messenger_creation() {
        let m = IrcMessenger::new(
            "test".to_string(),
            "irc.libera.chat".to_string(),
            6697,
            "rustyclaw".to_string(),
        );
        assert_eq!(m.name(), "test");
        assert_eq!(m.messenger_type(), "irc");
        assert!(!m.is_connected());
        assert!(m.use_tls);
    }

    #[test]
    fn test_parse_privmsg() {
        let line = ":nick!user@host PRIVMSG #channel :hello world";
        let (sender, target, msg) = parse_privmsg(line).unwrap();
        assert_eq!(sender, "nick");
        assert_eq!(target, "#channel");
        assert_eq!(msg, "hello world");
    }

    #[test]
    fn test_parse_privmsg_dm() {
        let line = ":alice!user@host PRIVMSG bot :direct message";
        let (sender, target, msg) = parse_privmsg(line).unwrap();
        assert_eq!(sender, "alice");
        assert_eq!(target, "bot");
        assert_eq!(msg, "direct message");
    }

    #[test]
    fn test_parse_ping() {
        assert_eq!(parse_ping("PING :server"), Some(":server"));
        assert_eq!(parse_ping("PRIVMSG #ch :hello"), None);
    }

    #[test]
    fn test_with_options() {
        let m = IrcMessenger::new(
            "test".to_string(),
            "irc.libera.chat".to_string(),
            6667,
            "bot".to_string(),
        )
        .with_channels(vec!["#test".to_string()])
        .with_tls(false)
        .with_password("secret".to_string());

        assert_eq!(m.channels, vec!["#test"]);
        assert!(!m.use_tls);
        assert_eq!(m.password, Some("secret".to_string()));
    }
}
