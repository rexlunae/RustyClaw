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

// ── Bottom bar data ─────────────────────────────────────────────────────────

/// Configuration for a directory entry in the directory selector.
#[derive(Clone, Debug, PartialEq)]
pub struct DirectoryOption {
    /// Full path to the directory.
    pub path: String,

    /// Display name (e.g., "~/projects" or "Home", or basename).
    pub display_name: String,

    /// Whether this is the current working directory.
    pub is_selected: bool,
}

/// State for the working directory selector.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DirectorySelectorState {
    /// Current working directory path.
    pub current_path: Option<String>,

    /// Display name for current directory (e.g., "~/workspace").
    pub current_display: Option<String>,

    /// Available directory options (favorites, recent, etc.).
    pub available_directories: Vec<DirectoryOption>,

    /// Whether the directory selector is expanded/open.
    pub is_expanded: bool,

    /// If set, shows an error message (e.g., "Permission denied").
    pub error: Option<String>,
}

/// Complete bottom bar state: everything needed to render the input area,
/// model/provider selectors, and working directory selector.
///
/// Does not include the live text input value — that's owned by the client's
/// local state (a `Signal<String>` in Dioxus, a `State<String>` in iocraft).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct BottomBarData {
    /// Composer state: processing, provider, model.
    pub composer: ComposerData,

    /// Working directory selector state.
    pub directory_selector: DirectorySelectorState,
}
