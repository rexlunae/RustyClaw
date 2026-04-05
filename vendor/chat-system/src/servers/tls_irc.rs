//! TLS-enabled IRC listener implementation.

use crate::server::{ChatListener, MessageHandler};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

/// A TLS-enabled TCP listener that speaks the IRC protocol.
///
/// Wraps a [`rustls::ServerConfig`] and performs TLS termination before handing
/// off to the same IRC message-handling logic used by
/// [`IrcListener`](super::IrcListener).
///
/// # Example
///
/// ```rust,no_run
/// use chat_system::server::Server;
/// use chat_system::servers::TlsIrcListener;
/// use chat_system::ChatServer;
/// use std::sync::Arc;
///
/// # #[tokio::main] async fn main() -> anyhow::Result<()> {
/// let tls_config = rustls::ServerConfig::builder()
///     .with_no_client_auth()
///     .with_single_cert(/* certs */ vec![], /* key */ todo!())?;
///
/// let mut server = Server::new("secure-echo")
///     .add_listener(TlsIrcListener::new("0.0.0.0:6697", Arc::new(tls_config)));
///
/// server.run(|msg| async move {
///     Ok(Some(format!("echo: {}", msg.content)))
/// }).await?;
/// # Ok(()) }
/// ```
pub struct TlsIrcListener {
    address: String,
    acceptor: TlsAcceptor,
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
}

impl TlsIrcListener {
    /// Create a new [`TlsIrcListener`] that will bind to `address` and terminate
    /// TLS using the provided [`rustls::ServerConfig`].
    pub fn new(address: impl Into<String>, config: Arc<rustls::ServerConfig>) -> Self {
        Self {
            address: address.into(),
            acceptor: TlsAcceptor::from(config),
            shutdown_tx: None,
        }
    }
}

#[async_trait]
impl ChatListener for TlsIrcListener {
    fn address(&self) -> &str {
        &self.address
    }

    fn protocol(&self) -> &str {
        "irc+tls"
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
        tracing::info!(address = %self.address, "IRC+TLS listener bound");
        self.shutdown_tx = Some(shutdown_tx);

        let acceptor = self.acceptor.clone();

        tokio::spawn(async move {
            let _alive = alive;

            loop {
                tokio::select! {
                    result = listener.accept() => {
                        match result {
                            Ok((stream, peer)) => {
                                tracing::debug!(%peer, "IRC+TLS listener: new connection");
                                let h = Arc::clone(&handler);
                                let acc = acceptor.clone();
                                tokio::spawn(async move {
                                    match acc.accept(stream).await {
                                        Ok(tls_stream) => {
                                            if let Err(e) = super::irc::handle_connection(tls_stream, h).await {
                                                tracing::warn!("IRC+TLS connection error: {e}");
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!("TLS handshake error from {peer}: {e}");
                                        }
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::warn!("IRC+TLS listener accept error: {e}");
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
