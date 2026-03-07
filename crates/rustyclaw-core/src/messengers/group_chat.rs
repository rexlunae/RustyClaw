//! Group chat support with isolation and activation modes.
//!
//! Provides configuration and logic for how the agent behaves in group
//! conversations (as opposed to 1:1 DMs). Mirrors OpenClaw's group chat
//! features:
//!
//! - **Activation modes**: How the agent decides to respond in a group.
//! - **Isolation modes**: Whether group conversations share state or are
//!   isolated per-group.

use serde::{Deserialize, Serialize};
use tracing::debug;

/// How the agent is activated in a group chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationMode {
    /// Always respond to every message in the group.
    Always,
    /// Only respond when mentioned by name or @-tag.
    Mention,
    /// Only respond when a specific prefix/command is used (e.g., "!claw").
    Prefix,
    /// Never respond in groups (DM only).
    Never,
}

impl Default for ActivationMode {
    fn default() -> Self {
        Self::Mention
    }
}

/// How group conversations are isolated from each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationMode {
    /// Each group gets its own conversation history and context.
    PerGroup,
    /// All groups share the same conversation history.
    Shared,
    /// Each user in each group gets their own context.
    PerUser,
}

impl Default for IsolationMode {
    fn default() -> Self {
        Self::PerGroup
    }
}

/// Group chat configuration for a messenger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupChatConfig {
    /// Whether group chat support is enabled.
    #[serde(default)]
    pub enabled: bool,

    /// How the agent is activated in groups.
    #[serde(default)]
    pub activation: ActivationMode,

    /// Command prefix for Prefix activation mode (e.g., "!claw").
    #[serde(default = "default_prefix")]
    pub prefix: String,

    /// How conversations are isolated.
    #[serde(default)]
    pub isolation: IsolationMode,

    /// Maximum number of messages to keep in group context.
    #[serde(default = "default_max_context")]
    pub max_context_messages: usize,

    /// Allowed group IDs (empty = all groups allowed).
    #[serde(default)]
    pub allowed_groups: Vec<String>,

    /// Blocked group IDs.
    #[serde(default)]
    pub blocked_groups: Vec<String>,

    /// Whether to include sender names in the context (helps the model
    /// understand who is speaking).
    #[serde(default = "default_true")]
    pub include_sender_names: bool,
}

fn default_prefix() -> String {
    "!claw".to_string()
}

fn default_max_context() -> usize {
    50
}

fn default_true() -> bool {
    true
}

impl Default for GroupChatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            activation: ActivationMode::default(),
            prefix: default_prefix(),
            isolation: IsolationMode::default(),
            max_context_messages: default_max_context(),
            allowed_groups: Vec::new(),
            blocked_groups: Vec::new(),
            include_sender_names: true,
        }
    }
}

impl GroupChatConfig {
    /// Check if a group is allowed.
    pub fn is_group_allowed(&self, group_id: &str) -> bool {
        if self.blocked_groups.contains(&group_id.to_string()) {
            return false;
        }
        if self.allowed_groups.is_empty() {
            return true;
        }
        self.allowed_groups.contains(&group_id.to_string())
    }

    /// Check if the agent should respond to a message in a group.
    pub fn should_respond(&self, message: &str, agent_name: &str) -> bool {
        if !self.enabled {
            return false;
        }

        match self.activation {
            ActivationMode::Always => true,
            ActivationMode::Never => false,
            ActivationMode::Mention => {
                let lower = message.to_lowercase();
                let name_lower = agent_name.to_lowercase();
                lower.contains(&name_lower)
                    || lower.contains(&format!("@{}", name_lower))
            }
            ActivationMode::Prefix => {
                message.starts_with(&self.prefix)
            }
        }
    }

    /// Generate a session key for isolation.
    pub fn session_key(&self, group_id: &str, user_id: Option<&str>) -> String {
        match self.isolation {
            IsolationMode::PerGroup => format!("group:{}", group_id),
            IsolationMode::Shared => "shared".to_string(),
            IsolationMode::PerUser => {
                if let Some(uid) = user_id {
                    format!("group:{}:user:{}", group_id, uid)
                } else {
                    format!("group:{}", group_id)
                }
            }
        }
    }

    /// Strip the prefix from a message (for Prefix activation mode).
    pub fn strip_prefix<'a>(&self, message: &'a str) -> &'a str {
        if self.activation == ActivationMode::Prefix {
            message
                .strip_prefix(&self.prefix)
                .map(|s| s.trim_start())
                .unwrap_or(message)
        } else {
            message
        }
    }
}

/// Format a group message with sender info for the model context.
pub fn format_group_message(sender_name: &str, message: &str, include_sender: bool) -> String {
    if include_sender {
        format!("[{}]: {}", sender_name, message)
    } else {
        message.to_string()
    }
}

/// Generate a group context key from messenger + group ID.
pub fn group_context_key(messenger_name: &str, group_id: &str) -> String {
    debug!(messenger = %messenger_name, group = %group_id, "Generating group context key");
    format!("{}:{}", messenger_name, group_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = GroupChatConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.activation, ActivationMode::Mention);
        assert_eq!(config.isolation, IsolationMode::PerGroup);
        assert_eq!(config.prefix, "!claw");
        assert!(config.include_sender_names);
    }

    #[test]
    fn test_should_respond_mention() {
        let config = GroupChatConfig {
            enabled: true,
            activation: ActivationMode::Mention,
            ..Default::default()
        };

        assert!(config.should_respond("Hey @rustyclaw help me", "RustyClaw"));
        assert!(config.should_respond("rustyclaw what do you think?", "RustyClaw"));
        assert!(!config.should_respond("Just chatting with friends", "RustyClaw"));
    }

    #[test]
    fn test_should_respond_prefix() {
        let config = GroupChatConfig {
            enabled: true,
            activation: ActivationMode::Prefix,
            prefix: "!claw".to_string(),
            ..Default::default()
        };

        assert!(config.should_respond("!claw help me", "RustyClaw"));
        assert!(!config.should_respond("Hey rustyclaw", "RustyClaw"));
    }

    #[test]
    fn test_should_respond_always() {
        let config = GroupChatConfig {
            enabled: true,
            activation: ActivationMode::Always,
            ..Default::default()
        };

        assert!(config.should_respond("anything at all", "RustyClaw"));
    }

    #[test]
    fn test_should_respond_never() {
        let config = GroupChatConfig {
            enabled: true,
            activation: ActivationMode::Never,
            ..Default::default()
        };

        assert!(!config.should_respond("@rustyclaw please", "RustyClaw"));
    }

    #[test]
    fn test_should_respond_disabled() {
        let config = GroupChatConfig {
            enabled: false,
            activation: ActivationMode::Always,
            ..Default::default()
        };

        assert!(!config.should_respond("anything", "RustyClaw"));
    }

    #[test]
    fn test_group_allowed() {
        let config = GroupChatConfig {
            allowed_groups: vec!["group1".to_string(), "group2".to_string()],
            ..Default::default()
        };

        assert!(config.is_group_allowed("group1"));
        assert!(!config.is_group_allowed("group3"));
    }

    #[test]
    fn test_group_blocked() {
        let config = GroupChatConfig {
            blocked_groups: vec!["spam".to_string()],
            ..Default::default()
        };

        assert!(!config.is_group_allowed("spam"));
        assert!(config.is_group_allowed("general"));
    }

    #[test]
    fn test_session_key_per_group() {
        let config = GroupChatConfig {
            isolation: IsolationMode::PerGroup,
            ..Default::default()
        };
        assert_eq!(config.session_key("g123", Some("u456")), "group:g123");
    }

    #[test]
    fn test_session_key_per_user() {
        let config = GroupChatConfig {
            isolation: IsolationMode::PerUser,
            ..Default::default()
        };
        assert_eq!(
            config.session_key("g123", Some("u456")),
            "group:g123:user:u456"
        );
    }

    #[test]
    fn test_session_key_shared() {
        let config = GroupChatConfig {
            isolation: IsolationMode::Shared,
            ..Default::default()
        };
        assert_eq!(config.session_key("g123", Some("u456")), "shared");
    }

    #[test]
    fn test_strip_prefix() {
        let config = GroupChatConfig {
            activation: ActivationMode::Prefix,
            prefix: "!claw".to_string(),
            ..Default::default()
        };
        assert_eq!(config.strip_prefix("!claw help me"), "help me");
        assert_eq!(config.strip_prefix("no prefix"), "no prefix");
    }

    #[test]
    fn test_format_group_message() {
        assert_eq!(
            format_group_message("Alice", "Hello!", true),
            "[Alice]: Hello!"
        );
        assert_eq!(format_group_message("Alice", "Hello!", false), "Hello!");
    }
}
