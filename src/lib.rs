#[cfg(feature = "tui")]
pub mod action;
#[cfg(feature = "tui")]
pub mod app;
pub mod args;
pub mod commands;
pub mod config;
pub mod cron;
pub mod daemon;
#[cfg(feature = "tui")]
pub mod dialogs;
pub mod gateway;
pub mod memory;
pub mod messengers;
#[cfg(feature = "tui")]
pub mod onboard;
#[cfg(feature = "tui")]
pub mod pages;
#[cfg(feature = "tui")]
pub mod panes;
pub mod process_manager;
pub mod providers;
pub mod retry;
pub mod sandbox;
pub mod secrets;
pub mod sessions;
pub mod skills;
pub mod soul;
pub mod streaming;
pub mod theme;
pub mod tools;
#[cfg(feature = "tui")]
pub mod tui;

// Re-export messenger types at crate root for convenience
pub use messengers::{Message, Messenger, MessengerManager, SendOptions};
