//! # chat-system
//!
//! A multi-protocol async chat crate for Rust.  Provides a **single unified
//! [`Messenger`] trait** for IRC, Matrix, Discord, Telegram, Slack, Signal,
//! WhatsApp, Microsoft Teams, Google Chat, iMessage, Webhook, and Console —
//! with full rich-text support for every platform's native format.
//!
//! The primary way to use this crate is through the **generic interface**:
//! [`MessengerConfig`] is a serde-tagged enum that selects the backend at
//! runtime, so the protocol is just a field in your config file rather than a
//! compile-time choice.
//!
//! ---
//!
//! ## Quick start — generic interface
//!
//! ```rust,no_run
//! use chat_system::{GenericMessenger, Messenger, MessengerConfig, PresenceStatus};
//! use chat_system::config::IrcConfig;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Build (or deserialize) a config — the protocol is just a field.
//!     let config = MessengerConfig::Irc(IrcConfig {
//!         name: "my-bot".into(),
//!         server: "irc.libera.chat".into(),
//!         port: 6697,
//!         nick: "my-bot".into(),
//!         channels: vec!["#rust".into()],
//!         tls: true,
//!     });
//!
//!     // GenericMessenger implements Messenger — swap the config to change protocol.
//!     let mut client = GenericMessenger::new(config);
//!     client.initialize().await?;
//!
//!     // Presence status (no-op on platforms that don't support it)
//!     client.set_status(PresenceStatus::Online).await?;
//!
//!     // Text status / custom status message
//!     client.set_text_status("Building something with Rust 🦀").await?;
//!
//!     client.send_message("#rust", "Hello from chat-system!").await?;
//!
//!     // Receive messages
//!     for msg in client.receive_messages().await? {
//!         println!("[{}] {}: {}", msg.channel.as_deref().unwrap_or("?"), msg.sender, msg.content);
//!         // Each message may carry reactions (populated on platforms that support them)
//!         if let Some(reactions) = &msg.reactions {
//!             for r in reactions { println!("  {} × {}", r.emoji, r.count); }
//!         }
//!     }
//!
//!     client.disconnect().await?;
//!     Ok(())
//! }
//! ```
//!
//! ### Loading the config from a file
//!
//! Because [`MessengerConfig`] derives `serde::Deserialize`, any
//! serde-compatible source works.
//!
//! **TOML** (`config.toml`):
//!
//! ```toml
//! protocol = "discord"
//! name     = "my-bot"
//! token    = "Bot TOKEN_HERE"
//! ```
//!
//! ```rust,ignore
//! # use chat_system::{GenericMessenger, Messenger, MessengerConfig};
//! # #[tokio::main] async fn main() -> anyhow::Result<()> {
//! let toml_str = std::fs::read_to_string("config.toml")?;
//! let config: MessengerConfig = toml::from_str(&toml_str)?;
//! let mut client = GenericMessenger::new(config);
//! client.initialize().await?;
//! # Ok(()) }
//! ```
//!
//! **JSON** (`config.json`):
//!
//! ```json
//! {"protocol":"telegram","name":"my-bot","token":"BOT_TOKEN"}
//! ```
//!
//! ```rust,no_run
//! # use chat_system::{GenericMessenger, Messenger, MessengerConfig};
//! # #[tokio::main] async fn main() -> anyhow::Result<()> {
//! let json_str = std::fs::read_to_string("config.json")?;
//! let config: MessengerConfig = serde_json::from_str(&json_str)?;
//! let mut client = GenericMessenger::new(config);
//! client.initialize().await?;
//! # Ok(()) }
//! ```
//!
//! ---
//!
//! ## Multi-platform with `MessengerManager`
//!
//! [`MessengerManager`] holds a collection of [`Messenger`] instances and
//! dispatches to all of them at once.
//!
//! ```rust,no_run
//! use chat_system::{GenericMessenger, Messenger, MessengerConfig, MessengerManager};
//! use chat_system::config::{DiscordConfig, TelegramConfig};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let mut mgr = MessengerManager::new()
//!         .add(GenericMessenger::new(MessengerConfig::Discord(DiscordConfig {
//!             name: "discord".into(),
//!             token: std::env::var("DISCORD_TOKEN")?,
//!         })))
//!         .add(GenericMessenger::new(MessengerConfig::Telegram(TelegramConfig {
//!             name: "telegram".into(),
//!             token: std::env::var("TELEGRAM_TOKEN")?,
//!         })));
//!     mgr.initialize_all().await?;
//!
//!     // Broadcast to every connected platform
//!     mgr.broadcast("#general", "Hello from all platforms!").await;
//!
//!     // Receive from every platform in one call
//!     for msg in mgr.receive_all().await? {
//!         println!("[{}] {}: {}", msg.channel.as_deref().unwrap_or("?"), msg.sender, msg.content);
//!     }
//!
//!     mgr.disconnect_all().await?;
//!     Ok(())
//! }
//! ```
//!
//! ---
//!
//! ## Reactions
//!
//! ```rust,no_run
//! # use chat_system::{GenericMessenger, Messenger, MessengerConfig};
//! # use chat_system::config::SlackConfig;
//! # #[tokio::main] async fn main() -> anyhow::Result<()> {
//! # let mut client = GenericMessenger::new(MessengerConfig::Slack(SlackConfig { name: "s".into(), token: "t".into() }));
//! # client.initialize().await?;
//! // Add a reaction (no-op on platforms that don't support it)
//! client.add_reaction("msg-id-123", "#general", "👍").await?;
//! client.remove_reaction("msg-id-123", "#general", "👍").await?;
//! # Ok(()) }
//! ```
//!
//! Incoming messages expose reactions via [`Message::reactions`]:
//!
//! ```rust,no_run
//! # use chat_system::{GenericMessenger, Messenger, MessengerConfig};
//! # use chat_system::config::SlackConfig;
//! # #[tokio::main] async fn main() -> anyhow::Result<()> {
//! # let client = GenericMessenger::new(MessengerConfig::Slack(SlackConfig { name: "s".into(), token: "t".into() }));
//! for msg in client.receive_messages().await? {
//!     if let Some(reactions) = &msg.reactions {
//!         for r in reactions {
//!             println!("{}: {} ({})", msg.id, r.emoji, r.count);
//!         }
//!     }
//! }
//! # Ok(()) }
//! ```
//!
//! ---
//!
//! ## Profile pictures
//!
//! ```rust,no_run
//! # use chat_system::{GenericMessenger, Messenger, MessengerConfig};
//! # use chat_system::config::DiscordConfig;
//! # #[tokio::main] async fn main() -> anyhow::Result<()> {
//! # let client = GenericMessenger::new(MessengerConfig::Discord(DiscordConfig { name: "d".into(), token: "t".into() }));
//! // Retrieve a user's profile picture URL (returns None if not supported)
//! if let Some(url) = client.get_profile_picture("user-id-123").await? {
//!     println!("Avatar: {url}");
//! }
//!
//! // Update the bot's own profile picture
//! client.set_profile_picture("https://example.com/avatar.png").await?;
//! # Ok(()) }
//! ```
//!
//! ---
//!
//! ## Text status / custom status message
//!
//! ```rust,no_run
//! # use chat_system::{GenericMessenger, Messenger, MessengerConfig, PresenceStatus};
//! # use chat_system::config::SlackConfig;
//! # #[tokio::main] async fn main() -> anyhow::Result<()> {
//! # let client = GenericMessenger::new(MessengerConfig::Slack(SlackConfig { name: "s".into(), token: "t".into() }));
//! // Presence indicator (Online / Away / Busy / Invisible / Offline)
//! client.set_status(PresenceStatus::Busy).await?;
//!
//! // Text status shown next to the username (Slack, Discord, …)
//! client.set_text_status("In a meeting 📅").await?;
//! # Ok(()) }
//! ```
//!
//! ---
//!
//! ## Replies
//!
//! Use [`SendOptions`] to reply to a specific message:
//!
//! ```rust,no_run
//! # use chat_system::{GenericMessenger, Messenger, MessengerConfig, SendOptions};
//! # use chat_system::config::DiscordConfig;
//! # #[tokio::main] async fn main() -> anyhow::Result<()> {
//! # let client = GenericMessenger::new(MessengerConfig::Discord(DiscordConfig { name: "d".into(), token: "t".into() }));
//! client.send_message_with_options(SendOptions {
//!     recipient: "#general",
//!     content: "Thanks for the message!",
//!     reply_to: Some("original-message-id"),
//!     ..Default::default()
//! }).await?;
//! # Ok(()) }
//! ```
//!
//! Incoming reply messages expose the parent via [`Message::reply_to`].
//!
//! ---
//!
//! ## Search
//!
//! ```rust,no_run
//! # use chat_system::{GenericMessenger, Messenger, MessengerConfig, SearchQuery};
//! # use chat_system::config::SlackConfig;
//! # #[tokio::main] async fn main() -> anyhow::Result<()> {
//! # let client = GenericMessenger::new(MessengerConfig::Slack(SlackConfig { name: "s".into(), token: "t".into() }));
//! let results = client.search_messages(SearchQuery {
//!     text: "deploy".into(),
//!     channel: Some("#ops".into()),
//!     limit: Some(20),
//!     ..Default::default()
//! }).await?;
//! for msg in results {
//!     println!("{}: {}", msg.sender, msg.content);
//! }
//! # Ok(()) }
//! ```
//!
//! ---
//!
//! ## Rich text
//!
//! ```rust,no_run
//! use chat_system::{RichText, RichTextNode};
//!
//! let msg = RichText(vec![
//!     RichTextNode::Bold(vec![RichTextNode::Plain("Hello".into())]),
//!     RichTextNode::Plain(", world! ".into()),
//!     RichTextNode::Link {
//!         url: "https://example.com".into(),
//!         text: vec![RichTextNode::Plain("click".into())],
//!     },
//! ]);
//!
//! println!("{}", msg.to_discord_markdown());
//! println!("{}", msg.to_telegram_html());
//! println!("{}", msg.to_slack_mrkdwn());
//! println!("{}", msg.to_irc_formatted());
//! ```
//!
//! ---
//!
//! ## Channel capabilities
//!
//! Every [`ChannelType`] exposes its feature set via [`ChannelType::descriptor`]:
//!
//! ```rust
//! use chat_system::ChannelType;
//!
//! let caps = ChannelType::Slack.descriptor().capabilities;
//! assert!(caps.supports_reactions);
//! assert!(caps.supports_threads);
//!
//! for ct in ChannelType::ALL {
//!     println!("{:14} reactions={} threads={}", ct.display_name(),
//!         ct.descriptor().capabilities.supports_reactions,
//!         ct.descriptor().capabilities.supports_threads);
//! }
//! ```
//!
//! ---
//!
//! ## Protocol-specific clients
//!
//! When you need access to protocol-specific features not covered by the generic
//! interface, you can construct the concrete messenger type directly:
//!
//! ```rust,no_run
//! use chat_system::messengers::IrcMessenger;
//! use chat_system::Messenger;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let mut client = IrcMessenger::new("my-bot", "irc.libera.chat", 6697, "my-bot")
//!         .with_tls(true)
//!         .with_channels(vec!["#rust"]);
//!     client.initialize().await?;
//!     client.send_message("#rust", "Hello, IRC!").await?;
//!     client.disconnect().await?;
//!     Ok(())
//! }
//! ```

pub mod channel_type;
pub mod config;
pub mod markdown;
pub mod message;
pub mod messenger;
pub mod messengers;
pub mod rich_text;
pub mod server;
pub mod servers;

pub use channel_type::{ChannelCapabilities, ChannelDescriptor, ChannelType, InboundMode};
pub use config::{GenericMessenger, GenericServer, MessengerConfig, ServerConfig};
pub use markdown::{chunk_markdown_html, markdown_to_slack, markdown_to_telegram_html};
pub use message::{MediaAttachment, Message, Reaction, SendOptions};
pub use messenger::{Messenger, MessengerManager, PresenceStatus, SearchQuery};
pub use rich_text::{RichText, RichTextNode};
pub use server::{ChatListener, ChatServer, MessageHandler, Server};
pub use servers::IrcListener;
