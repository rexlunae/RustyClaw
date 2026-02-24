// ── User prompt data types ──────────────────────────────────────────────────
//
// Protocol-level types for the `ask_user` tool. These are used by the gateway
// to send structured prompts and receive responses. Client crates implement
// the actual UI rendering.

use serde::{Deserialize, Serialize};

/// A structured prompt sent by the agent via the `ask_user` tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserPrompt {
    /// Unique prompt ID (matches tool call ID).
    pub id: String,
    /// The question or instruction text.
    pub title: String,
    /// Optional longer description.
    #[serde(default)]
    pub description: Option<String>,
    /// The type of input requested.
    pub prompt_type: PromptType,
}

/// The different input types the agent can request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PromptType {
    /// Pick exactly one option from a list.
    Select {
        options: Vec<PromptOption>,
        #[serde(default)]
        default: Option<usize>,
    },
    /// Pick zero or more options from a list.
    MultiSelect {
        options: Vec<PromptOption>,
        #[serde(default)]
        defaults: Vec<usize>,
    },
    /// Yes/No confirmation.
    Confirm {
        #[serde(default = "default_true")]
        default: bool,
    },
    /// Free text input.
    TextInput {
        #[serde(default)]
        placeholder: Option<String>,
        #[serde(default)]
        default: Option<String>,
    },
    /// Multiple named text fields.
    Form {
        fields: Vec<FormField>,
    },
}

fn default_true() -> bool {
    true
}

/// A selectable option with a label and optional description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptOption {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
}

/// A text field in a Form prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormField {
    pub name: String,
    pub label: String,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub required: bool,
}

/// Typed response value from a user prompt.
///
/// Each variant matches a `PromptType` input kind so the response is
/// statically typed end-to-end — no `serde_json::Value` anywhere.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PromptResponseValue {
    /// Free-text input or a single selected option label.
    Text(String),
    /// Yes / No confirmation.
    Confirm(bool),
    /// Zero or more selected option labels.
    Selected(Vec<String>),
    /// Form field name→value pairs.
    Form(Vec<(String, String)>),
}

/// The user's response to a prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserPromptResponse {
    pub id: String,
    /// Whether the user dismissed the prompt (Esc).
    pub dismissed: bool,
    /// The typed response value.
    pub value: PromptResponseValue,
}
