//! IRC listener implementation.

use crate::message::Message;
use crate::server::{ChatListener, MessageHandler};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

static IRC_MSG_COUNTER: AtomicU64 = AtomicU64::new(0);

// ── IrcListener ───────────────────────────────────────────────────────────────

/// A TCP listener that speaks the IRC protocol.
///
/// When started, it binds the configured address, accepts incoming connections,
/// parses IRC messages, invokes the message handler, and sends replies back in
/// IRC wire format.  Multiple `IrcListener` instances can be attached to a
/// single [`Server`](crate::server::Server) so that it is reachable on several
/// ports simultaneously.
///
/// ```rust,no_run
/// use chat_system::server::Server;
/// use chat_system::servers::IrcListener;
///
/// # #[tokio::main] async fn main() -> anyhow::Result<()> {
/// let mut server = Server::new("my-irc")
///     .add_listener(IrcListener::new("0.0.0.0:6667"))
///     .add_listener(IrcListener::new("0.0.0.0:6697"));
/// // server.run(handler).await?;
/// # Ok(()) }
/// ```
pub struct IrcListener {
    address: String,
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
}

impl IrcListener {
    /// Create a new [`IrcListener`] that will bind to `address` (e.g.
    /// `"127.0.0.1:6667"`).
    pub fn new(address: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            shutdown_tx: None,
        }
    }
}

/// Handle a single IRC connection: perform the handshake, parse `PRIVMSG`
/// lines, invoke the handler, and write replies.
///
/// Generic over the stream type so it can be used with both plain TCP and TLS
/// connections.
pub(super) async fn handle_connection(
    stream: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    handler: MessageHandler,
) -> Result<()> {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    let mut nick = String::new();
    let mut user_seen = false;
    let mut registered = false;

    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix("NICK ") {
            nick = rest.trim().to_string();
        } else if line.starts_with("USER ") {
            user_seen = true;
        } else if line.starts_with("PING ") {
            let token = line.trim_start_matches("PING ");
            writer
                .write_all(format!("PONG {}\r\n", token).as_bytes())
                .await?;
        } else if let Some(rest) = line.strip_prefix("PRIVMSG ") {
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() == 2 {
                let target = parts[0];
                let content = parts[1].trim_start_matches(':');
                let msg = Message {
                    id: format!(
                        "irc-{}-{}",
                        chrono::Utc::now().timestamp_millis(),
                        IRC_MSG_COUNTER.fetch_add(1, Ordering::Relaxed),
                    ),
                    sender: nick.clone(),
                    content: content.to_string(),
                    timestamp: chrono::Utc::now().timestamp(),
                    channel: Some(target.to_string()),
                    reply_to: None,
                    media: None,
                    is_direct: !target.starts_with('#'),
                    reactions: None,
                };
                if let Ok(Some(reply)) = handler(msg).await {
                    let response =
                        format!(":server!server@localhost PRIVMSG {} :{}\r\n", target, reply);
                    writer.write_all(response.as_bytes()).await?;
                }
            }
        } else if line == "QUIT" || line.starts_with("QUIT ") {
            break;
        }

        if !registered && !nick.is_empty() && user_seen {
            writer
                .write_all(format!(":localhost 001 {} :Welcome\r\n", nick).as_bytes())
                .await?;
            registered = true;
        }
    }
    Ok(())
}

#[async_trait]
impl ChatListener for IrcListener {
    fn address(&self) -> &str {
        &self.address
    }

    fn protocol(&self) -> &str {
        "irc"
    }

    async fn start(
        &mut self,
        handler: MessageHandler,
        alive: tokio::sync::mpsc::Sender<()>,
    ) -> Result<()> {
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
        let listener = TcpListener::bind(&self.address).await?;
        // Update to the actual bound address (useful when port 0 is requested).
        self.address = listener.local_addr()?.to_string();
        tracing::info!(address = %self.address, "IRC listener bound");
        self.shutdown_tx = Some(shutdown_tx);

        tokio::spawn(async move {
            // Hold `alive` — when this task exits, the sender is dropped,
            // signalling the server that this listener has stopped.
            let _alive = alive;

            loop {
                tokio::select! {
                    result = listener.accept() => {
                        match result {
                            Ok((stream, peer)) => {
                                tracing::debug!(%peer, "IRC listener: new connection");
                                let h = Arc::clone(&handler);
                                tokio::spawn(async move {
                                    if let Err(e) = handle_connection(stream, h).await {
                                        tracing::warn!("IRC connection error: {e}");
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::warn!("IRC listener accept error: {e}");
                                break;
                            }
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                    }
                }
            }
        });

        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        if let Some(tx) = &self.shutdown_tx {
            let _ = tx.send(true);
        }
        Ok(())
    }
}
