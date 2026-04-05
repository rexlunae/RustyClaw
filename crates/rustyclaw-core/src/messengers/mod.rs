//! Messenger system — thin re-export layer over the `chat-system` crate.
//!
//! All platform implementations (IRC, Discord, Telegram, Slack, Teams, Google Chat,
//! iMessage, Console, Webhook, Matrix, WhatsApp, Signal) are provided by the
//! [`chat_system`] crate.  This module re-exports the public API so that the rest
//! of `rustyclaw-core` can continue to use `crate::messengers::*` paths.
//!
//! ## Pending PRs to chat-system
//!
//! The following CLI-based implementations are **not yet in chat-system** and are
//! kept locally until the corresponding PRs are merged:
//!
//! - `matrix_cli` — Matrix over plain HTTP (no SDK), feature `matrix-cli`
//! - `telegram_cli` — Telegram Bot API, feature `telegram-cli`
//! - `discord_cli` — Discord REST API, feature `discord-cli`
//! - `slack_cli` — Slack Web API, feature `slack-cli`
//!
//! Utility modules (`media`, `streaming`, `group_chat`) are also kept locally
//! until they are contributed upstream to `chat-system`.

// ── Core types and traits from chat-system ──────────────────────────────────

pub use chat_system::message::{MediaAttachment, Message, SendOptions};
pub use chat_system::messenger::{Messenger, MessengerManager, PresenceStatus, SearchQuery};

// ── Concrete messenger implementations from chat-system ─────────────────────

pub use chat_system::messengers::{
    ConsoleMessenger, DiscordMessenger, GoogleChatMessenger, IMessageMessenger, IrcMessenger,
    SlackMessenger, TeamsMessenger, TelegramMessenger, WebhookMessenger,
};

#[cfg(feature = "matrix")]
pub use chat_system::messengers::MatrixMessenger;

#[cfg(feature = "signal-cli")]
pub use chat_system::messengers::SignalCliMessenger;

#[cfg(feature = "whatsapp")]
pub use chat_system::messengers::WhatsAppMessenger;

// ── Utility modules (kept locally; pending PRs to chat-system) ──────────────

pub mod group_chat;
pub mod media;
pub mod streaming;

pub use group_chat::GroupChatConfig;
pub use media::{MediaConfig, MediaType};
pub use streaming::{FlushAction, StreamBuffer, StreamConfig, StreamStrategy};

// ── CLI-based messengers (pending PRs to chat-system) ───────────────────────

#[cfg(feature = "matrix-cli")]
pub mod matrix_cli;
#[cfg(feature = "matrix-cli")]
pub use matrix_cli::{MatrixCliMessenger, MatrixDmConfig};

#[cfg(feature = "telegram-cli")]
pub mod telegram_cli;
#[cfg(feature = "telegram-cli")]
pub use telegram_cli::TelegramCliMessenger;

#[cfg(feature = "discord-cli")]
pub mod discord_cli;
#[cfg(feature = "discord-cli")]
pub use discord_cli::DiscordCliMessenger;

#[cfg(feature = "slack-cli")]
pub mod slack_cli;
#[cfg(feature = "slack-cli")]
pub use slack_cli::SlackCliMessenger;
