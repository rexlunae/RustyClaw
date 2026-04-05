//! Protocol-specific messenger implementations.

mod console;
mod discord;
mod google_chat;
mod imessage;
mod irc;
mod slack;
mod teams;
mod telegram;
mod webhook;

pub use console::ConsoleMessenger;
pub use discord::DiscordMessenger;
pub use google_chat::GoogleChatMessenger;
pub use imessage::IMessageMessenger;
pub use irc::IrcMessenger;
pub use slack::SlackMessenger;
pub use teams::TeamsMessenger;
pub use telegram::TelegramMessenger;
pub use webhook::WebhookMessenger;

#[cfg(feature = "matrix")]
mod matrix;
#[cfg(feature = "matrix")]
pub use matrix::MatrixMessenger;

#[cfg(feature = "signal-cli")]
mod signal_cli;
#[cfg(feature = "signal-cli")]
pub use signal_cli::SignalCliMessenger;

#[cfg(feature = "whatsapp")]
mod whatsapp;
#[cfg(feature = "whatsapp")]
pub use whatsapp::WhatsAppMessenger;
