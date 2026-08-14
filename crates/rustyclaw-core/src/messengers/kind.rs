//! The `messenger_type` field, typed.
//!
//! The unit variants are the in-tree vocabulary — one per id in
//! [`super::setup::KINDS`], plus the legacy `signal-cli` spelling, which is
//! kept as its own variant so a config file that says `signal-cli` round-trips
//! byte-for-byte instead of being silently rewritten to `signal`.
//!
//! [`MessengerKind::Other`] is not a dumping ground: the kind vocabulary is
//! genuinely open — plugins register kinds at runtime
//! ([`super::MessengerRegistry`]), a config written by a newer build may name
//! a kind this one has never heard of, and a fresh account starts with the
//! empty string. `Other` carries exactly those, and the registry decides what
//! they mean.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A messenger backend kind, as written in `messenger_type` in config.
///
/// Serializes as its string id (`"google_chat"`, not `GoogleChat`), so config
/// files and wire frames are unchanged from when this was a bare `String`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum MessengerKind {
    Telegram,
    Discord,
    Slack,
    Matrix,
    Irc,
    Signal,
    /// Legacy spelling of [`MessengerKind::Signal`]; same backend.
    SignalCli,
    WhatsApp,
    GoogleChat,
    Teams,
    IMessage,
    Webhook,
    Console,
    /// A kind outside the in-tree vocabulary: a plugin-registered kind, an id
    /// from a newer build, a retired id (`matrix-cli`), or the empty string of
    /// an account not yet configured.
    Other(String),
}

impl MessengerKind {
    /// The id as written in config — the inverse of `From<&str>`.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Telegram => "telegram",
            Self::Discord => "discord",
            Self::Slack => "slack",
            Self::Matrix => "matrix",
            Self::Irc => "irc",
            Self::Signal => "signal",
            Self::SignalCli => "signal-cli",
            Self::WhatsApp => "whatsapp",
            Self::GoogleChat => "google_chat",
            Self::Teams => "teams",
            Self::IMessage => "imessage",
            Self::Webhook => "webhook",
            Self::Console => "console",
            Self::Other(id) => id,
        }
    }

    /// The id the registry and schema tables index by — folds the legacy
    /// `signal-cli` spelling into `signal`.
    pub fn canonical_str(&self) -> &str {
        match self {
            Self::SignalCli => "signal",
            other => other.as_str(),
        }
    }

    /// Whether this is the empty id an account carries before its type is
    /// chosen — distinct from a real (if unrecognized) kind.
    pub fn is_unset(&self) -> bool {
        matches!(self, Self::Other(id) if id.is_empty())
    }
}

impl Default for MessengerKind {
    /// The unset state, matching the empty string this field defaulted to as
    /// a `String`.
    fn default() -> Self {
        Self::Other(String::new())
    }
}

impl From<&str> for MessengerKind {
    fn from(id: &str) -> Self {
        match id {
            "telegram" => Self::Telegram,
            "discord" => Self::Discord,
            "slack" => Self::Slack,
            "matrix" => Self::Matrix,
            "irc" => Self::Irc,
            "signal" => Self::Signal,
            "signal-cli" => Self::SignalCli,
            "whatsapp" => Self::WhatsApp,
            "google_chat" => Self::GoogleChat,
            "teams" => Self::Teams,
            "imessage" => Self::IMessage,
            "webhook" => Self::Webhook,
            "console" => Self::Console,
            other => Self::Other(other.to_string()),
        }
    }
}

impl From<String> for MessengerKind {
    fn from(id: String) -> Self {
        match Self::from(id.as_str()) {
            // Keep the allocation the caller already made.
            Self::Other(_) => Self::Other(id),
            known => known,
        }
    }
}

impl From<MessengerKind> for String {
    fn from(kind: MessengerKind) -> Self {
        match kind {
            MessengerKind::Other(id) => id,
            known => known.as_str().to_string(),
        }
    }
}

impl fmt::Display for MessengerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq<str> for MessengerKind {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for MessengerKind {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_in_tree_kind_round_trips_through_its_id() {
        for spec in crate::messengers::setup::KINDS {
            let kind = MessengerKind::from(spec.id.as_ref());
            assert!(
                !matches!(kind, MessengerKind::Other(_)),
                "KINDS id '{}' has no MessengerKind variant",
                spec.id
            );
            assert_eq!(kind.as_str(), spec.id, "round-trip changed the id");
        }
    }

    #[test]
    fn legacy_signal_cli_spelling_survives_round_trip_but_canonicalizes() {
        let kind = MessengerKind::from("signal-cli");
        assert_eq!(kind, MessengerKind::SignalCli);
        assert_eq!(kind.as_str(), "signal-cli");
        assert_eq!(kind.canonical_str(), "signal");
    }

    #[test]
    fn unknown_and_empty_ids_are_other() {
        assert_eq!(
            MessengerKind::from("acme_chat"),
            MessengerKind::Other("acme_chat".to_string())
        );
        assert!(MessengerKind::default().is_unset());
        assert!(!MessengerKind::from("acme_chat").is_unset());
    }

    #[test]
    fn serde_form_is_the_bare_string() {
        let json = serde_json::to_string(&MessengerKind::GoogleChat).unwrap();
        assert_eq!(json, "\"google_chat\"");
        let back: MessengerKind = serde_json::from_str("\"google_chat\"").unwrap();
        assert_eq!(back, MessengerKind::GoogleChat);
        let plugin: MessengerKind = serde_json::from_str("\"acme_chat\"").unwrap();
        assert_eq!(plugin, MessengerKind::Other("acme_chat".to_string()));
    }
}
