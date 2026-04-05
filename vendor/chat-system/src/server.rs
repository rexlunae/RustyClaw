//! [`ChatServer`] and [`ChatListener`] traits for server-side implementations,
//! plus the concrete protocol-agnostic [`Server`].
//!
//! A **server** is a named container of listeners.  It owns no address, port, or
//! protocol of its own — those belong to the listeners.  Different listeners may
//! speak entirely different protocols (e.g. IRC on one port, WebSocket on
//! another) while still feeding messages into the same server event loop.
//!
//! A **listener** is a single (protocol, address, port) combination.  It handles
//! all wire-protocol details: accepting connections, parsing inbound data into
//! [`Message`]s, calling the message handler, and formatting replies back in the
//! appropriate wire format.

use crate::message::Message;
use anyhow::Result;
use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

// ── MessageHandler ────────────────────────────────────────────────────────────

/// A type-erased, cloneable, async message handler.
///
/// Listeners receive one of these from the server and call it for every inbound
/// message.  The return value `Option<String>` is an optional plain-text reply
/// that the listener may format and send back in its wire protocol.
pub type MessageHandler = Arc<
    dyn Fn(Message) -> Pin<Box<dyn Future<Output = Result<Option<String>>> + Send>> + Send + Sync,
>;

/// Wrap a generic async closure into a [`MessageHandler`].
///
/// # Example
///
/// ```rust,ignore
/// use chat_system::server::into_handler;
///
/// let h = into_handler(|msg| async move {
///     println!("got: {}", msg.content);
///     Ok(Some("thanks!".into()))
/// });
/// ```
pub fn into_handler<F, Fut>(f: F) -> MessageHandler
where
    F: Fn(Message) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Option<String>>> + Send + 'static,
{
    Arc::new(move |msg| Box::pin(f(msg)))
}

// ── ChatListener ──────────────────────────────────────────────────────────────

/// A single network endpoint: one (protocol, address, port) combination.
///
/// Listeners handle all wire-protocol details.  When [`ChatListener::start`] is
/// called the listener binds its address, accepts connections, parses inbound
/// data into [`Message`]s, invokes the provided [`MessageHandler`], and sends
/// any replies back in the appropriate wire format.
#[async_trait]
pub trait ChatListener: Send + Sync {
    /// The address this listener is (or will be) bound to.
    fn address(&self) -> &str;

    /// The wire protocol this listener speaks (e.g. `"irc"`).
    fn protocol(&self) -> &str;

    /// Start accepting connections and processing messages.
    ///
    /// The `handler` is called for every inbound message; the optional `String`
    /// return value is a reply that the listener formats into its wire protocol.
    ///
    /// The `alive` sender should be held (cloned) by every spawned task.  When
    /// all clones are dropped the server knows this listener has fully stopped.
    async fn start(
        &mut self,
        handler: MessageHandler,
        alive: tokio::sync::mpsc::Sender<()>,
    ) -> Result<()>;

    /// Stop accepting new connections and shut down all tasks.
    async fn shutdown(&mut self) -> Result<()>;
}

// ── ChatServer ────────────────────────────────────────────────────────────────

/// A protocol-agnostic chat server.
///
/// A server is defined by its *name* and the set of [`ChatListener`]s attached
/// to it.  It has no inherent address, port, or protocol — those are properties
/// of the individual listeners.
#[async_trait]
pub trait ChatServer: Send + Sync {
    /// Human-readable name of this server.
    fn name(&self) -> &str;

    /// Snapshot of all currently attached listeners (for introspection).
    fn listeners(&self) -> Vec<&dyn ChatListener>;

    /// Start all listeners and run the server event loop.
    ///
    /// Blocks until all listeners have exited (either through
    /// [`ChatServer::shutdown`] or because they finished naturally).
    async fn run<F, Fut>(&mut self, handler: F) -> Result<()>
    where
        F: Fn(Message) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Option<String>>> + Send + 'static;

    /// Shut down all listeners, causing [`ChatServer::run`] to return.
    async fn shutdown(&mut self) -> Result<()>;
}

// ── Server (concrete) ─────────────────────────────────────────────────────────

/// The standard, protocol-agnostic [`ChatServer`] implementation.
///
/// Construct one with [`Server::new`], attach any number of listeners with
/// [`Server::add_listener`], then call [`ChatServer::run`]:
///
/// ```rust,no_run
/// use chat_system::server::Server;
/// use chat_system::servers::IrcListener;
/// use chat_system::ChatServer;
///
/// # #[tokio::main] async fn main() -> anyhow::Result<()> {
/// let mut server = Server::new("my-server")
///     .add_listener(IrcListener::new("0.0.0.0:6667"))
///     .add_listener(IrcListener::new("0.0.0.0:6697"));
///
/// server.run(|msg| async move {
///     println!("{}: {}", msg.sender, msg.content);
///     Ok(Some(format!("echo: {}", msg.content)))
/// }).await?;
/// # Ok(()) }
/// ```
///
/// With the `tls` feature enabled, you can mix plaintext and TLS listeners:
///
/// ```rust,ignore
/// use chat_system::servers::{IrcListener, TlsIrcListener};
///
/// let server = Server::new("my-server")
///     .add_listener(IrcListener::new("0.0.0.0:6667"))
///     .add_listener(TlsIrcListener::new("0.0.0.0:6697", tls_config));
/// ```
pub struct Server {
    name: String,
    listeners: Vec<Box<dyn ChatListener>>,
}

impl Server {
    /// Create a new empty server with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            listeners: Vec::new(),
        }
    }

    /// Attach a listener.  The listener may speak any protocol.
    ///
    /// Listeners must be added **before** calling [`ChatServer::run`].
    pub fn add_listener(mut self, listener: impl ChatListener + 'static) -> Self {
        self.listeners.push(Box::new(listener));
        self
    }

    /// Attach an already-boxed listener.
    ///
    /// This is useful when you already have a `Box<dyn ChatListener>`, for
    /// example from a [`ListenerConfig::build`](crate::config::ListenerConfig::build) call.
    /// Listeners must be added **before** calling [`ChatServer::run`].
    pub fn add_boxed_listener(mut self, listener: Box<dyn ChatListener>) -> Self {
        self.listeners.push(listener);
        self
    }
}

#[async_trait]
impl ChatServer for Server {
    fn name(&self) -> &str {
        &self.name
    }

    fn listeners(&self) -> Vec<&dyn ChatListener> {
        self.listeners.iter().map(|l| l.as_ref()).collect()
    }

    async fn run<F, Fut>(&mut self, handler: F) -> Result<()>
    where
        F: Fn(Message) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Option<String>>> + Send + 'static,
    {
        let handler: MessageHandler = into_handler(handler);

        // Each listener holds a clone of `alive_tx`.  When every clone is
        // dropped (all listener tasks have exited) the `alive_rx.recv()` below
        // returns `None` and `run()` completes.
        let (alive_tx, mut alive_rx) = tokio::sync::mpsc::channel::<()>(1);

        for listener in &mut self.listeners {
            listener.start(handler.clone(), alive_tx.clone()).await?;
        }

        // Drop our own clone so the channel closes when all listeners stop.
        drop(alive_tx);

        // Block until all listeners have exited.
        let _ = alive_rx.recv().await;

        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        for listener in &mut self.listeners {
            listener.shutdown().await?;
        }
        Ok(())
    }
}
