//! Messenger facade built on top of the shared `chat-system` crate.
//!
//! All messenger implementations are now in the `chat-system` crate.
//! This module re-exports them for backwards compatibility.

pub use chat_system::messengers::{
    ConsoleMessenger, DiscordMessenger, GoogleChatMessenger, IMessageMessenger, IrcMessenger,
    SlackMessenger, TeamsMessenger, TelegramMessenger, WebhookMessenger,
};
pub use chat_system::{
    GenericMessenger, MediaAttachment, Message, Messenger, MessengerManager, PresenceStatus,
    SendOptions,
};

#[cfg(feature = "matrix")]
pub use chat_system::messengers::{MatrixDmConfig, MatrixMessenger};

#[cfg(feature = "whatsapp")]
pub use chat_system::messengers::WhatsAppMessenger;

#[cfg(feature = "signal-cli")]
pub use chat_system::messengers::SignalCliMessenger;
