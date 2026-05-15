//! Component data for the composer (input bar + model selector).
//!
//! The composer is the text area where the user types messages,
//! plus the provider/model dropdown bar above it.  This module
//! provides the shared data types that both clients use to render
//! the same composer UI.

/// Everything the composer bar needs to render.
///
/// Does not include the text input's live value — that's owned by
/// the client's local state (a `Signal<String>` in Dioxus, a
/// `State<String>` in iocraft).  This struct describes the *props*
/// that change from the parent: processing state and model selection.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ComposerData {
    /// Whether the gateway is currently processing a response.
    pub is_processing: bool,

    /// Active provider identifier (e.g. "openrouter").
    pub current_provider: Option<String>,

    /// Active model identifier (e.g. "gpt-4o").
    pub current_model: Option<String>,
}
