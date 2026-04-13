//! Messenger facade built on top of the shared `chat-system` crate.

pub use chat_system::messengers::{
    ConsoleMessenger, DiscordMessenger, GoogleChatMessenger, IMessageMessenger, IrcMessenger,
    SlackMessenger, TeamsMessenger, TelegramMessenger, WebhookMessenger,
};
pub use chat_system::{
    GenericMessenger, MediaAttachment, Message, Messenger, MessengerManager, PresenceStatus,
    SendOptions,
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

// NOTE: telegram-cli, discord-cli, and slack-cli features removed.
// Use TelegramMessenger, DiscordMessenger, SlackMessenger from chat-system instead.
// MatrixCliMessenger kept temporarily for state_dir, allowed_chats, dm_config features
// that need to be upstreamed to chat-system.
