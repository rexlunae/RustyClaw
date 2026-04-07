//! Messenger facade built on top of the shared `chat-system` crate.

pub use chat_system::{
    MediaAttachment, Message, Messenger, MessengerManager, PresenceStatus, SendOptions,
};
pub use chat_system::messengers::{
    ConsoleMessenger, DiscordMessenger, GoogleChatMessenger, IMessageMessenger, IrcMessenger,
    SlackMessenger, TeamsMessenger, TelegramMessenger, WebhookMessenger,
};

pub mod group_chat;
pub mod media;
pub mod streaming;
pub use group_chat::GroupChatConfig;
pub use media::{MediaConfig, MediaType};
pub use streaming::{StreamBuffer, StreamConfig, StreamStrategy};

#[cfg(feature = "matrix")]
pub use chat_system::messengers::MatrixMessenger;

#[cfg(feature = "whatsapp")]
pub use chat_system::messengers::WhatsAppMessenger;

#[cfg(feature = "signal-cli")]
pub use chat_system::messengers::SignalCliMessenger;

#[cfg(feature = "matrix-cli")]
mod matrix_cli;
#[cfg(feature = "matrix-cli")]
pub use matrix_cli::{MatrixCliMessenger, MatrixDmConfig};

#[cfg(feature = "telegram-cli")]
mod telegram_cli;
#[cfg(feature = "telegram-cli")]
pub use telegram_cli::TelegramCliMessenger;

#[cfg(feature = "discord-cli")]
mod discord_cli;
#[cfg(feature = "discord-cli")]
pub use discord_cli::DiscordCliMessenger;

#[cfg(feature = "slack-cli")]
mod slack_cli;
#[cfg(feature = "slack-cli")]
pub use slack_cli::SlackCliMessenger;
