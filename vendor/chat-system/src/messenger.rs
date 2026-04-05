//! The [`Messenger`] trait and [`MessengerManager`].

use crate::message::{Message, SendOptions};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// The presence/availability status of a messenger account or bot.
///
/// Not every platform supports every variant; unsupported values fall back to
/// the closest equivalent or are silently ignored via the default no-op
/// implementation of [`Messenger::set_status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PresenceStatus {
    /// Fully available and accepting messages.
    Online,
    /// Temporarily away (e.g. idle, away message set).
    Away,
    /// Do-not-disturb / busy — notifications may be suppressed.
    Busy,
    /// Signed in but appearing as offline to other users.
    Invisible,
    /// Fully offline / disconnected.
    Offline,
}

/// Structured query for searching messages.
///
/// All fields are optional; only the fields provided are used as filters.
/// Platforms that do not support a particular filter silently ignore it.
///
/// The struct is serde-serializable so it can be loaded from config files or
/// forwarded over APIs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchQuery {
    /// Free-text search string (empty string matches all messages).
    #[serde(default)]
    pub text: String,
    /// Restrict the search to a particular channel or conversation ID.
    #[serde(default)]
    pub channel: Option<String>,
    /// Restrict to messages from a specific sender ID / username.
    #[serde(default)]
    pub from: Option<String>,
    /// Maximum number of results to return.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Return only messages sent before this Unix timestamp (exclusive).
    #[serde(default)]
    pub before_timestamp: Option<i64>,
    /// Return only messages sent after this Unix timestamp (exclusive).
    #[serde(default)]
    pub after_timestamp: Option<i64>,
}

/// A unified interface for chat platform clients.
#[async_trait]
pub trait Messenger: Send + Sync {
    fn name(&self) -> &str;
    fn messenger_type(&self) -> &str;
    async fn initialize(&mut self) -> Result<()>;
    async fn send_message(&self, recipient: &str, content: &str) -> Result<String>;
    async fn send_message_with_options(&self, opts: SendOptions<'_>) -> Result<String> {
        self.send_message(opts.recipient, opts.content).await
    }
    async fn receive_messages(&self) -> Result<Vec<Message>>;
    fn is_connected(&self) -> bool;
    async fn disconnect(&mut self) -> Result<()>;
    async fn set_typing(&self, _channel: &str, _typing: bool) -> Result<()> {
        Ok(())
    }
    /// Set the bot's own presence/availability status.
    ///
    /// Platforms that do not support a particular [`PresenceStatus`] value, or
    /// that have no presence API at all, may ignore this call.  The default
    /// implementation is a no-op so that existing messenger implementations
    /// are unaffected.
    async fn set_status(&self, _status: PresenceStatus) -> Result<()> {
        Ok(())
    }

    /// Add an emoji reaction to a message.
    ///
    /// `message_id` is the platform message ID, `channel` is the channel or
    /// conversation it belongs to, and `emoji` is the reaction emoji (Unicode
    /// character or platform shortcode).
    ///
    /// Platforms that do not support reactions return `Ok(())` silently via
    /// this default implementation.
    async fn add_reaction(&self, _message_id: &str, _channel: &str, _emoji: &str) -> Result<()> {
        Ok(())
    }

    /// Remove an emoji reaction from a message.
    ///
    /// Has the same signature as [`add_reaction`](Messenger::add_reaction).
    /// Platforms that do not support reactions return `Ok(())` silently.
    async fn remove_reaction(&self, _message_id: &str, _channel: &str, _emoji: &str) -> Result<()> {
        Ok(())
    }

    /// Retrieve the profile-picture URL for a user.
    ///
    /// Returns `Ok(None)` on platforms that do not expose profile pictures or
    /// when the user has no picture set.
    async fn get_profile_picture(&self, _user_id: &str) -> Result<Option<String>> {
        Ok(None)
    }

    /// Update the bot's own profile picture.
    ///
    /// `url` may be an HTTP URL or a `file://` path depending on what the
    /// platform accepts.  Platforms that do not support this operation silently
    /// return `Ok(())`.
    async fn set_profile_picture(&self, _url: &str) -> Result<()> {
        Ok(())
    }

    /// Set the bot's text status / custom status message.
    ///
    /// This is distinct from [`set_status`](Messenger::set_status), which
    /// controls the presence indicator (online/away/busy/…).  A text status is
    /// a short human-readable string displayed next to the user's name on
    /// platforms that support it (e.g. Slack, Discord).
    ///
    /// Platforms that do not support text statuses silently return `Ok(())`.
    async fn set_text_status(&self, _text: &str) -> Result<()> {
        Ok(())
    }

    /// Search for messages matching `query`.
    ///
    /// Returns an empty `Vec` on platforms that do not support server-side
    /// search.  Results are returned in an unspecified order unless the
    /// platform guarantees one.
    async fn search_messages(&self, _query: SearchQuery) -> Result<Vec<Message>> {
        Ok(Vec::new())
    }
}

/// Manages multiple [`Messenger`] instances.
pub struct MessengerManager {
    messengers: Vec<Box<dyn Messenger>>,
}

impl MessengerManager {
    pub fn new() -> Self {
        Self {
            messengers: Vec::new(),
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn add(mut self, messenger: impl Messenger + 'static) -> Self {
        self.messengers.push(Box::new(messenger));
        self
    }

    pub fn add_boxed(mut self, messenger: Box<dyn Messenger>) -> Self {
        self.messengers.push(messenger);
        self
    }

    pub async fn initialize_all(&mut self) -> Result<()> {
        for m in &mut self.messengers {
            m.initialize().await?;
        }
        Ok(())
    }

    pub async fn disconnect_all(&mut self) -> Result<()> {
        for m in &mut self.messengers {
            m.disconnect().await?;
        }
        Ok(())
    }

    pub async fn receive_all(&self) -> Result<Vec<Message>> {
        let mut all = Vec::new();
        for m in &self.messengers {
            match m.receive_messages().await {
                Ok(mut msgs) => all.append(&mut msgs),
                Err(e) => tracing::warn!(messenger = %m.name(), "receive error: {e}"),
            }
        }
        Ok(all)
    }

    pub async fn broadcast(
        &self,
        recipient: impl AsRef<str>,
        content: impl AsRef<str>,
    ) -> Vec<Result<String>> {
        let mut results = Vec::new();
        for m in &self.messengers {
            results.push(m.send_message(recipient.as_ref(), content.as_ref()).await);
        }
        results
    }

    pub fn messengers(&self) -> &[Box<dyn Messenger>] {
        &self.messengers
    }

    pub fn get(&self, name: impl AsRef<str>) -> Option<&dyn Messenger> {
        self.messengers
            .iter()
            .find(|m| m.name() == name.as_ref())
            .map(|b| b.as_ref())
    }
}

impl Default for MessengerManager {
    fn default() -> Self {
        Self::new()
    }
}
