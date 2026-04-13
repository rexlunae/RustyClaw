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
pub use chat_system::messengers::{MatrixMessenger, MatrixDmConfig};

#[cfg(feature = "whatsapp")]
pub use chat_system::messengers::WhatsAppMessenger;

#[cfg(feature = "signal-cli")]
pub use chat_system::messengers::SignalCliMessenger;
