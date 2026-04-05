//! Config-driven generic client and server types.
//!
//! [`MessengerConfig`] and [`ServerConfig`] are serde-tagged enums whose `protocol`
//! field selects the backend.  They can be deserialized directly from TOML, JSON,
//! or any other serde-compatible format, making them suitable for config files.
//!
//! # Client example (TOML)
//!
//! ```toml
//! protocol = "irc"
//! name     = "my-bot"
//! server   = "irc.libera.chat"
//! port     = 6697
//! nick     = "my-bot"
//! channels = ["#rust"]
//! tls      = true
//! ```
//!
//! ```rust,no_run
//! use chat_system::config::{IrcConfig, MessengerConfig};
//! use chat_system::{GenericMessenger, Messenger};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let config = MessengerConfig::Irc(IrcConfig {
//!         name: "bot".into(),
//!         server: "irc.libera.chat".into(),
//!         port: 6697,
//!         nick: "my-bot".into(),
//!         channels: vec!["#rust".into()],
//!         tls: true,
//!     });
//!     let mut client = GenericMessenger::new(config);
//!     client.initialize().await?;
//!     client.send_message("#rust", "Hello!").await?;
//!     client.disconnect().await?;
//!     Ok(())
//! }
//! ```

use crate::message::{Message, SendOptions};
use crate::messenger::{Messenger, PresenceStatus, SearchQuery};
use crate::server::ChatServer;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ── per-protocol client config structs ────────────────────────────────────────

/// Configuration for an IRC client connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrcConfig {
    pub name: String,
    pub server: String,
    #[serde(default = "default_irc_port")]
    pub port: u16,
    pub nick: String,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub tls: bool,
}
fn default_irc_port() -> u16 {
    6667
}

/// Configuration for a Discord bot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordConfig {
    pub name: String,
    pub token: String,
}

/// Configuration for a Telegram bot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub name: String,
    pub token: String,
}

/// Configuration for a Slack bot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackConfig {
    pub name: String,
    pub token: String,
}

/// Configuration for Microsoft Teams.
///
/// Use `webhook_url` for legacy incoming-webhook mode, or `token` + `team_id`
/// + `channel_id` for Microsoft Graph mode with inbound polling support.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamsConfig {
    pub name: String,
    #[serde(default)]
    pub webhook_url: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
}

/// Configuration for Google Chat.
///
/// Use `webhook_url` for incoming-webhook mode, or `token` + `space_id` for
/// authenticated Google Chat API mode with inbound polling support.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleChatConfig {
    pub name: String,
    #[serde(default)]
    pub webhook_url: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub space_id: Option<String>,
}

/// Configuration for the console (stdin/stdout) messenger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleConfig {
    pub name: String,
}

/// Configuration for an outbound HTTP webhook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub name: String,
    pub url: String,
}

/// Configuration for the iMessage messenger (macOS only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IMessageConfig {
    pub name: String,
}

#[cfg(feature = "matrix")]
/// Configuration for a Matrix client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixConfig {
    pub name: String,
    pub homeserver: String,
    pub username: String,
    pub password: String,
}

#[cfg(feature = "signal-cli")]
/// Configuration for a Signal CLI messenger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalCliConfig {
    pub name: String,
    pub phone_number: String,
    #[serde(default = "default_signal_cli_path")]
    pub cli_path: String,
}
#[cfg(feature = "signal-cli")]
fn default_signal_cli_path() -> String {
    "signal-cli".to_string()
}

#[cfg(feature = "whatsapp")]
/// Configuration for a WhatsApp messenger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsAppConfig {
    pub name: String,
    /// Path to the SQLite session database (e.g. `"whatsapp.db"`).
    pub db_path: String,
}

// ── MessengerConfig ────────────────────────────────────────────────────────────

/// Protocol-selecting messenger configuration.
///
/// The `protocol` field (the serde tag) identifies the backend.  Deserializing
/// from a config file that contains `protocol = "irc"` will produce
/// `MessengerConfig::Irc(IrcConfig { … })`.
///
/// Call [`MessengerConfig::build`] to obtain a concrete [`Messenger`], or wrap it
/// in a [`GenericMessenger`] which itself implements [`Messenger`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "protocol", rename_all = "lowercase")]
pub enum MessengerConfig {
    Irc(IrcConfig),
    Discord(DiscordConfig),
    Telegram(TelegramConfig),
    Slack(SlackConfig),
    Teams(TeamsConfig),
    #[serde(rename = "googlechat")]
    GoogleChat(GoogleChatConfig),
    Console(ConsoleConfig),
    Webhook(WebhookConfig),
    #[serde(rename = "imessage")]
    IMessage(IMessageConfig),
    #[cfg(feature = "matrix")]
    Matrix(MatrixConfig),
    #[cfg(feature = "signal-cli")]
    #[serde(rename = "signal")]
    SignalCli(SignalCliConfig),
    #[cfg(feature = "whatsapp")]
    WhatsApp(WhatsAppConfig),
}

impl MessengerConfig {
    /// The human-readable name for this messenger instance.
    pub fn name(&self) -> &str {
        match self {
            Self::Irc(c) => &c.name,
            Self::Discord(c) => &c.name,
            Self::Telegram(c) => &c.name,
            Self::Slack(c) => &c.name,
            Self::Teams(c) => &c.name,
            Self::GoogleChat(c) => &c.name,
            Self::Console(c) => &c.name,
            Self::Webhook(c) => &c.name,
            Self::IMessage(c) => &c.name,
            #[cfg(feature = "matrix")]
            Self::Matrix(c) => &c.name,
            #[cfg(feature = "signal-cli")]
            Self::SignalCli(c) => &c.name,
            #[cfg(feature = "whatsapp")]
            Self::WhatsApp(c) => &c.name,
        }
    }

    /// The protocol identifier string (matches the serde tag value).
    pub fn protocol_name(&self) -> &'static str {
        match self {
            Self::Irc(_) => "irc",
            Self::Discord(_) => "discord",
            Self::Telegram(_) => "telegram",
            Self::Slack(_) => "slack",
            Self::Teams(_) => "teams",
            Self::GoogleChat(_) => "googlechat",
            Self::Console(_) => "console",
            Self::Webhook(_) => "webhook",
            Self::IMessage(_) => "imessage",
            #[cfg(feature = "matrix")]
            Self::Matrix(_) => "matrix",
            #[cfg(feature = "signal-cli")]
            Self::SignalCli(_) => "signal",
            #[cfg(feature = "whatsapp")]
            Self::WhatsApp(_) => "whatsapp",
        }
    }

    /// Construct a concrete [`Messenger`] from this config.
    ///
    /// The returned messenger has **not** been initialized; call
    /// [`Messenger::initialize`] before use, or use [`GenericMessenger`] which
    /// does this automatically.
    pub fn build(&self) -> Result<Box<dyn Messenger>> {
        use crate::messengers::*;
        let m: Box<dyn Messenger> = match self {
            Self::Irc(c) => Box::new(
                IrcMessenger::new(&c.name, &c.server, c.port, &c.nick)
                    .with_channels(c.channels.clone())
                    .with_tls(c.tls),
            ),
            Self::Discord(c) => Box::new(DiscordMessenger::new(&c.name, &c.token)),
            Self::Telegram(c) => Box::new(TelegramMessenger::new(&c.name, &c.token)),
            Self::Slack(c) => Box::new(SlackMessenger::new(&c.name, &c.token)),
            Self::Teams(c) => match (&c.webhook_url, &c.token, &c.team_id, &c.channel_id) {
                (_, Some(token), Some(team_id), Some(channel_id)) => Box::new(
                    TeamsMessenger::new_graph(&c.name, token, team_id, channel_id),
                ),
                (Some(webhook_url), _, _, _) => Box::new(TeamsMessenger::new(&c.name, webhook_url)),
                _ => anyhow::bail!(
                    "Teams config requires either webhook_url or token + team_id + channel_id"
                ),
            },
            Self::GoogleChat(c) => match (&c.webhook_url, &c.token, &c.space_id) {
                (_, Some(token), Some(space_id)) => {
                    Box::new(GoogleChatMessenger::new_api(&c.name, token, space_id))
                }
                (Some(webhook_url), _, _) => Box::new(GoogleChatMessenger::new(&c.name, webhook_url)),
                _ => anyhow::bail!(
                    "Google Chat config requires either webhook_url or token + space_id"
                ),
            },
            Self::Console(c) => Box::new(ConsoleMessenger::new(&c.name)),
            Self::Webhook(c) => Box::new(WebhookMessenger::new(&c.name, &c.url)),
            Self::IMessage(c) => Box::new(IMessageMessenger::new(&c.name)),
            #[cfg(feature = "matrix")]
            Self::Matrix(c) => Box::new(MatrixMessenger::new(
                &c.name,
                &c.homeserver,
                &c.username,
                &c.password,
            )),
            #[cfg(feature = "signal-cli")]
            Self::SignalCli(c) => Box::new(
                SignalCliMessenger::new(&c.name, &c.phone_number).with_cli_path(&c.cli_path),
            ),
            #[cfg(feature = "whatsapp")]
            Self::WhatsApp(c) => Box::new(WhatsAppMessenger::new(&c.name, &c.db_path)),
        };
        Ok(m)
    }
}

// ── per-protocol server config structs ────────────────────────────────────────

/// Configuration for an IRC listener.
///
/// Each `IrcListenerConfig` represents a single TCP address the server will
/// accept IRC connections on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrcListenerConfig {
    /// TCP address to bind (e.g. `"0.0.0.0:6667"`).
    pub address: String,
}

#[typetag::serde(name = "irc")]
impl ListenerConfig for IrcListenerConfig {
    fn protocol(&self) -> &str {
        "irc"
    }

    fn address(&self) -> &str {
        &self.address
    }

    fn build(&self) -> Box<dyn crate::server::ChatListener> {
        Box::new(crate::servers::IrcListener::new(&self.address))
    }

    fn clone_box(&self) -> Box<dyn ListenerConfig> {
        Box::new(self.clone())
    }

    fn debug_fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

// ── ListenerConfig ────────────────────────────────────────────────────────────

/// Per-listener configuration trait.
///
/// Each protocol provides its own config struct that implements this trait.
/// The trait is serde-compatible via [`typetag`], so `Vec<Box<dyn
/// ListenerConfig>>` can be serialized and deserialized from JSON, TOML, etc.
///
/// # Implementing a custom listener config
///
/// ```rust,ignore
/// use chat_system::config::ListenerConfig;
///
/// #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
/// pub struct MyListenerConfig { pub address: String }
///
/// #[typetag::serde(name = "my-protocol")]
/// impl ListenerConfig for MyListenerConfig {
///     fn protocol(&self) -> &str { "my-protocol" }
///     fn address(&self) -> &str { &self.address }
///     fn build(&self) -> Box<dyn chat_system::server::ChatListener> { todo!() }
///     fn clone_box(&self) -> Box<dyn ListenerConfig> { Box::new(self.clone()) }
///     fn debug_fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
///         std::fmt::Debug::fmt(self, f)
///     }
/// }
/// ```
#[typetag::serde(tag = "protocol")]
pub trait ListenerConfig: Send + Sync {
    /// The wire protocol this listener speaks (e.g. `"irc"`).
    fn protocol(&self) -> &str;

    /// The address this listener will bind.
    fn address(&self) -> &str;

    /// Construct a concrete [`ChatListener`](crate::server::ChatListener) from
    /// this config.
    fn build(&self) -> Box<dyn crate::server::ChatListener>;

    /// Clone this config into a new boxed trait object.
    fn clone_box(&self) -> Box<dyn ListenerConfig>;

    /// Format this config for debug output.
    fn debug_fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result;
}

impl Clone for Box<dyn ListenerConfig> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl std::fmt::Debug for dyn ListenerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.debug_fmt(f)
    }
}

// ── ServerConfig ───────────────────────────────────────────────────────────────

/// Server configuration.
///
/// A server has a `name` and a list of listener configs.  Each listener may use
/// a different protocol; the server itself is protocol-agnostic.
///
/// # Example (JSON)
///
/// ```json
/// {
///   "name": "my-server",
///   "listeners": [
///     { "protocol": "irc", "address": "0.0.0.0:6667" },
///     { "protocol": "irc", "address": "0.0.0.0:6697" }
///   ]
/// }
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerConfig {
    pub name: String,
    pub listeners: Vec<Box<dyn ListenerConfig>>,
}

impl ServerConfig {
    /// The human-readable name for this server instance.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The listener configurations.
    pub fn listener_configs(&self) -> &[Box<dyn ListenerConfig>] {
        &self.listeners
    }
}

// ── GenericMessenger ───────────────────────────────────────────────────────────

/// A [`Messenger`] whose protocol is determined at runtime by a [`MessengerConfig`].
///
/// Construct with a config, call [`Messenger::initialize`] to establish the
/// connection (which also builds the inner backend), then use it like any other
/// [`Messenger`].
///
/// Because [`GenericMessenger`] implements [`Messenger`] it is a drop-in
/// replacement everywhere a `Box<dyn Messenger>` is accepted, including
/// [`MessengerManager`](crate::MessengerManager).
pub struct GenericMessenger {
    config: MessengerConfig,
    inner: Option<Box<dyn Messenger>>,
}

impl GenericMessenger {
    /// Create a new uninitialized [`GenericMessenger`] from a config.
    pub fn new(config: MessengerConfig) -> Self {
        Self {
            config,
            inner: None,
        }
    }

    /// Access the underlying config.
    pub fn config(&self) -> &MessengerConfig {
        &self.config
    }
}

#[async_trait]
impl Messenger for GenericMessenger {
    fn name(&self) -> &str {
        self.inner
            .as_ref()
            .map(|m| m.name())
            .unwrap_or_else(|| self.config.name())
    }

    fn messenger_type(&self) -> &str {
        self.inner
            .as_ref()
            .map(|m| m.messenger_type())
            .unwrap_or_else(|| self.config.protocol_name())
    }

    async fn initialize(&mut self) -> Result<()> {
        let mut built = self.config.build()?;
        built.initialize().await?;
        self.inner = Some(built);
        Ok(())
    }

    async fn send_message(&self, recipient: &str, content: &str) -> Result<String> {
        self.inner
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("GenericMessenger not initialized"))?
            .send_message(recipient, content)
            .await
    }

    async fn send_message_with_options(&self, opts: SendOptions<'_>) -> Result<String> {
        self.inner
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("GenericMessenger not initialized"))?
            .send_message_with_options(opts)
            .await
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        self.inner
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("GenericMessenger not initialized"))?
            .receive_messages()
            .await
    }

    fn is_connected(&self) -> bool {
        self.inner
            .as_ref()
            .map(|m| m.is_connected())
            .unwrap_or(false)
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(inner) = &mut self.inner {
            inner.disconnect().await?;
        }
        Ok(())
    }

    async fn set_typing(&self, channel: &str, typing: bool) -> Result<()> {
        if let Some(inner) = &self.inner {
            inner.set_typing(channel, typing).await
        } else {
            Ok(())
        }
    }

    async fn set_status(&self, status: PresenceStatus) -> Result<()> {
        if let Some(inner) = &self.inner {
            inner.set_status(status).await
        } else {
            Ok(())
        }
    }

    async fn add_reaction(&self, message_id: &str, channel: &str, emoji: &str) -> Result<()> {
        if let Some(inner) = &self.inner {
            inner.add_reaction(message_id, channel, emoji).await
        } else {
            Ok(())
        }
    }

    async fn remove_reaction(&self, message_id: &str, channel: &str, emoji: &str) -> Result<()> {
        if let Some(inner) = &self.inner {
            inner.remove_reaction(message_id, channel, emoji).await
        } else {
            Ok(())
        }
    }

    async fn get_profile_picture(&self, user_id: &str) -> Result<Option<String>> {
        if let Some(inner) = &self.inner {
            inner.get_profile_picture(user_id).await
        } else {
            Ok(None)
        }
    }

    async fn set_profile_picture(&self, url: &str) -> Result<()> {
        if let Some(inner) = &self.inner {
            inner.set_profile_picture(url).await
        } else {
            Ok(())
        }
    }

    async fn set_text_status(&self, text: &str) -> Result<()> {
        if let Some(inner) = &self.inner {
            inner.set_text_status(text).await
        } else {
            Ok(())
        }
    }

    async fn search_messages(&self, query: SearchQuery) -> Result<Vec<Message>> {
        if let Some(inner) = &self.inner {
            inner.search_messages(query).await
        } else {
            Ok(Vec::new())
        }
    }
}

// ── GenericServer ──────────────────────────────────────────────────────────────

/// A config-driven server.
///
/// Builds a [`Server`](crate::server::Server) from a [`ServerConfig`],
/// constructing the appropriate listeners for each entry in the config's
/// `listeners` list.
pub struct GenericServer {
    config: ServerConfig,
    inner: Option<crate::server::Server>,
}

impl GenericServer {
    /// Create a new [`GenericServer`] from a config.
    pub fn new(config: ServerConfig) -> Self {
        Self {
            config,
            inner: None,
        }
    }

    /// Access the underlying config.
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    fn build_inner(&self) -> crate::server::Server {
        let mut server = crate::server::Server::new(&self.config.name);
        for lc in &self.config.listeners {
            server = server.add_boxed_listener(lc.build());
        }
        server
    }
}

#[async_trait]
impl ChatServer for GenericServer {
    fn name(&self) -> &str {
        match &self.inner {
            Some(s) => s.name(),
            None => &self.config.name,
        }
    }

    fn listeners(&self) -> Vec<&dyn crate::server::ChatListener> {
        match &self.inner {
            Some(s) => s.listeners(),
            None => Vec::new(),
        }
    }

    async fn run<F, Fut>(&mut self, handler: F) -> Result<()>
    where
        F: Fn(Message) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Option<String>>> + Send + 'static,
    {
        if self.inner.is_none() {
            self.inner = Some(self.build_inner());
        }
        self.inner.as_mut().unwrap().run(handler).await
    }

    async fn shutdown(&mut self) -> Result<()> {
        if let Some(s) = &mut self.inner {
            s.shutdown().await
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messenger_config_roundtrip_json() {
        let cfg = MessengerConfig::Irc(IrcConfig {
            name: "bot".into(),
            server: "irc.libera.chat".into(),
            port: 6697,
            nick: "bot".into(),
            channels: vec!["#rust".into()],
            tls: true,
        });
        let json = serde_json::to_string(&cfg).unwrap();
        let decoded: MessengerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.protocol_name(), "irc");
        assert_eq!(decoded.name(), "bot");
    }

    #[test]
    fn messenger_config_deserialize_protocol_tag() {
        let json = r#"{"protocol":"discord","name":"d-bot","token":"tok123"}"#;
        let cfg: MessengerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.protocol_name(), "discord");
        assert_eq!(cfg.name(), "d-bot");
    }

    #[test]
    fn server_config_roundtrip_json() {
        let cfg = ServerConfig {
            name: "srv".into(),
            listeners: vec![Box::new(IrcListenerConfig {
                address: "0.0.0.0:6667".into(),
            })],
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let decoded: ServerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.name(), "srv");
        assert_eq!(decoded.listeners.len(), 1);
        assert_eq!(decoded.listeners[0].address(), "0.0.0.0:6667");
        assert_eq!(decoded.listeners[0].protocol(), "irc");
    }

    #[test]
    fn server_config_multi_listener_roundtrip_json() {
        let cfg = ServerConfig {
            name: "srv".into(),
            listeners: vec![
                Box::new(IrcListenerConfig {
                    address: "0.0.0.0:6667".into(),
                }),
                Box::new(IrcListenerConfig {
                    address: "0.0.0.0:6697".into(),
                }),
            ],
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let decoded: ServerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.listeners.len(), 2);
        assert_eq!(decoded.listeners[0].address(), "0.0.0.0:6667");
        assert_eq!(decoded.listeners[1].address(), "0.0.0.0:6697");
    }

    #[test]
    fn generic_messenger_name_before_init() {
        let cfg = MessengerConfig::Console(ConsoleConfig { name: "con".into() });
        let gm = GenericMessenger::new(cfg);
        assert_eq!(gm.name(), "con");
        assert_eq!(gm.messenger_type(), "console");
        assert!(!gm.is_connected());
    }

    #[test]
    fn generic_server_name_before_run() {
        let cfg = ServerConfig {
            name: "srv".into(),
            listeners: vec![Box::new(IrcListenerConfig {
                address: "127.0.0.1:7777".into(),
            })],
        };
        let gs = GenericServer::new(cfg);
        assert_eq!(gs.name(), "srv");
    }

    #[tokio::test]
    async fn generic_messenger_set_typing_before_init_is_ok() {
        let cfg = MessengerConfig::Console(ConsoleConfig { name: "con".into() });
        let gm = GenericMessenger::new(cfg);
        // Should be a no-op (not initialized yet), not an error.
        gm.set_typing("#general", true).await.unwrap();
        gm.set_typing("#general", false).await.unwrap();
    }

    #[tokio::test]
    async fn generic_messenger_set_typing_after_init_is_ok() {
        let cfg = MessengerConfig::Console(ConsoleConfig { name: "con".into() });
        let mut gm = GenericMessenger::new(cfg);
        gm.initialize().await.unwrap();
        gm.set_typing("#general", true).await.unwrap();
        gm.set_typing("#general", false).await.unwrap();
        gm.disconnect().await.unwrap();
    }

    #[tokio::test]
    async fn generic_messenger_set_status_before_init_is_ok() {
        let cfg = MessengerConfig::Console(ConsoleConfig { name: "con".into() });
        let gm = GenericMessenger::new(cfg);
        gm.set_status(PresenceStatus::Online).await.unwrap();
        gm.set_status(PresenceStatus::Away).await.unwrap();
        gm.set_status(PresenceStatus::Busy).await.unwrap();
        gm.set_status(PresenceStatus::Invisible).await.unwrap();
        gm.set_status(PresenceStatus::Offline).await.unwrap();
    }

    #[tokio::test]
    async fn generic_messenger_set_status_after_init_is_ok() {
        let cfg = MessengerConfig::Console(ConsoleConfig { name: "con".into() });
        let mut gm = GenericMessenger::new(cfg);
        gm.initialize().await.unwrap();
        gm.set_status(PresenceStatus::Online).await.unwrap();
        gm.set_status(PresenceStatus::Away).await.unwrap();
        gm.disconnect().await.unwrap();
    }

    #[test]
    fn presence_status_serde_roundtrip() {
        for status in [
            PresenceStatus::Online,
            PresenceStatus::Away,
            PresenceStatus::Busy,
            PresenceStatus::Invisible,
            PresenceStatus::Offline,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let decoded: PresenceStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, status);
        }
    }

    #[test]
    fn presence_status_json_values() {
        assert_eq!(
            serde_json::to_string(&PresenceStatus::Online).unwrap(),
            r#""online""#
        );
        assert_eq!(
            serde_json::to_string(&PresenceStatus::Away).unwrap(),
            r#""away""#
        );
        assert_eq!(
            serde_json::to_string(&PresenceStatus::Busy).unwrap(),
            r#""busy""#
        );
        assert_eq!(
            serde_json::to_string(&PresenceStatus::Invisible).unwrap(),
            r#""invisible""#
        );
        assert_eq!(
            serde_json::to_string(&PresenceStatus::Offline).unwrap(),
            r#""offline""#
        );
    }

    #[tokio::test]
    async fn generic_messenger_add_reaction_before_init_is_ok() {
        let cfg = MessengerConfig::Console(ConsoleConfig { name: "con".into() });
        let gm = GenericMessenger::new(cfg);
        gm.add_reaction("msg-1", "#general", "👍").await.unwrap();
    }

    #[tokio::test]
    async fn generic_messenger_add_reaction_after_init_is_ok() {
        let cfg = MessengerConfig::Console(ConsoleConfig { name: "con".into() });
        let mut gm = GenericMessenger::new(cfg);
        gm.initialize().await.unwrap();
        gm.add_reaction("msg-1", "#general", "👍").await.unwrap();
        gm.disconnect().await.unwrap();
    }

    #[tokio::test]
    async fn generic_messenger_remove_reaction_before_init_is_ok() {
        let cfg = MessengerConfig::Console(ConsoleConfig { name: "con".into() });
        let gm = GenericMessenger::new(cfg);
        gm.remove_reaction("msg-1", "#general", "👍").await.unwrap();
    }

    #[tokio::test]
    async fn generic_messenger_remove_reaction_after_init_is_ok() {
        let cfg = MessengerConfig::Console(ConsoleConfig { name: "con".into() });
        let mut gm = GenericMessenger::new(cfg);
        gm.initialize().await.unwrap();
        gm.remove_reaction("msg-1", "#general", "❤️").await.unwrap();
        gm.disconnect().await.unwrap();
    }

    #[tokio::test]
    async fn generic_messenger_get_profile_picture_before_init_returns_none() {
        let cfg = MessengerConfig::Console(ConsoleConfig { name: "con".into() });
        let gm = GenericMessenger::new(cfg);
        let pic = gm.get_profile_picture("alice").await.unwrap();
        assert!(pic.is_none());
    }

    #[tokio::test]
    async fn generic_messenger_get_profile_picture_after_init_returns_none() {
        let cfg = MessengerConfig::Console(ConsoleConfig { name: "con".into() });
        let mut gm = GenericMessenger::new(cfg);
        gm.initialize().await.unwrap();
        let pic = gm.get_profile_picture("bob").await.unwrap();
        assert!(pic.is_none());
        gm.disconnect().await.unwrap();
    }

    #[tokio::test]
    async fn generic_messenger_set_profile_picture_before_init_is_ok() {
        let cfg = MessengerConfig::Console(ConsoleConfig { name: "con".into() });
        let gm = GenericMessenger::new(cfg);
        gm.set_profile_picture("https://example.com/avatar.png")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn generic_messenger_set_profile_picture_after_init_is_ok() {
        let cfg = MessengerConfig::Console(ConsoleConfig { name: "con".into() });
        let mut gm = GenericMessenger::new(cfg);
        gm.initialize().await.unwrap();
        gm.set_profile_picture("https://example.com/avatar.png")
            .await
            .unwrap();
        gm.disconnect().await.unwrap();
    }

    #[tokio::test]
    async fn generic_messenger_set_text_status_before_init_is_ok() {
        let cfg = MessengerConfig::Console(ConsoleConfig { name: "con".into() });
        let gm = GenericMessenger::new(cfg);
        gm.set_text_status("Working from home 🏠").await.unwrap();
    }

    #[tokio::test]
    async fn generic_messenger_set_text_status_after_init_is_ok() {
        let cfg = MessengerConfig::Console(ConsoleConfig { name: "con".into() });
        let mut gm = GenericMessenger::new(cfg);
        gm.initialize().await.unwrap();
        gm.set_text_status("In a meeting").await.unwrap();
        gm.disconnect().await.unwrap();
    }

    #[tokio::test]
    async fn generic_messenger_search_messages_before_init_returns_empty() {
        let cfg = MessengerConfig::Console(ConsoleConfig { name: "con".into() });
        let gm = GenericMessenger::new(cfg);
        let results = gm
            .search_messages(SearchQuery {
                text: "hello".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn generic_messenger_search_messages_after_init_returns_empty() {
        let cfg = MessengerConfig::Console(ConsoleConfig { name: "con".into() });
        let mut gm = GenericMessenger::new(cfg);
        gm.initialize().await.unwrap();
        let results = gm
            .search_messages(SearchQuery {
                text: "rust".into(),
                channel: Some("#general".into()),
                limit: Some(10),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(results.is_empty());
        gm.disconnect().await.unwrap();
    }

    #[test]
    fn search_query_serde_roundtrip() {
        let q = SearchQuery {
            text: "hello world".into(),
            channel: Some("#rust".into()),
            from: Some("alice".into()),
            limit: Some(50),
            before_timestamp: Some(9_999_999),
            after_timestamp: Some(1_000_000),
        };
        let json = serde_json::to_string(&q).unwrap();
        let de: SearchQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(de.text, q.text);
        assert_eq!(de.channel, q.channel);
        assert_eq!(de.from, q.from);
        assert_eq!(de.limit, q.limit);
        assert_eq!(de.before_timestamp, q.before_timestamp);
        assert_eq!(de.after_timestamp, q.after_timestamp);
    }

    #[test]
    fn search_query_defaults() {
        let q: SearchQuery = serde_json::from_str(r#"{"text":"hi"}"#).unwrap();
        assert_eq!(q.text, "hi");
        assert!(q.channel.is_none());
        assert!(q.from.is_none());
        assert!(q.limit.is_none());
        assert!(q.before_timestamp.is_none());
        assert!(q.after_timestamp.is_none());
    }
}
