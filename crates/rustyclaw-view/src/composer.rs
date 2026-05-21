//! Component data for the composer (input bar + model selector).
//!
//! The composer is the text area where the user types messages,
//! plus the provider/model dropdown bar above it.  This module
//! provides the shared data types that both clients use to render
//! the same composer UI.

use std::path::Path;

/// An attachment that should be included with the next prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptAttachmentKind {
    File,
    Directory,
}

impl PromptAttachmentKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Directory => "Directory",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::File => "📄",
            Self::Directory => "📁",
        }
    }
}

/// A file or directory attached to the prompt builder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptAttachment {
    /// Attachment kind.
    pub kind: PromptAttachmentKind,

    /// Absolute or workspace-relative path.
    pub path: String,

    /// Short display label for the UI.
    pub display_name: String,
}

impl PromptAttachment {
    fn display_name_for_path(path: &str) -> String {
        Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_string())
            .unwrap_or_else(|| path.to_string())
    }

    pub fn file(path: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            kind: PromptAttachmentKind::File,
            path: path.into(),
            display_name: display_name.into(),
        }
    }

    pub fn directory(path: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            kind: PromptAttachmentKind::Directory,
            path: path.into(),
            display_name: display_name.into(),
        }
    }

    pub fn from_file_path(path: impl Into<String>) -> Self {
        let path = path.into();
        Self::file(path.clone(), Self::display_name_for_path(&path))
    }

    pub fn from_directory_path(path: impl Into<String>) -> Self {
        let path = path.into();
        Self::directory(path.clone(), Self::display_name_for_path(&path))
    }

    pub fn prompt_line(&self) -> String {
        format!("- {} {}: {}", self.kind.icon(), self.kind.label(), self.path)
    }
}

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

    /// Files/directories the user attached to the next prompt.
    pub attachments: Vec<PromptAttachment>,
}

impl ComposerData {
    pub fn attachment_count(&self) -> usize {
        self.attachments.len()
    }

    pub fn has_attachments(&self) -> bool {
        !self.attachments.is_empty()
    }
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

/// Build a prompt string that includes attached files/directories as context.
pub fn build_prompt_with_attachments(
    message: &str,
    attachments: &[PromptAttachment],
) -> String {
    if attachments.is_empty() {
        return message.to_string();
    }

    let attachment_block = attachments
        .iter()
        .map(PromptAttachment::prompt_line)
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "{}\n\n## Attached Context\n{}\n\nUse the attached files/directories as part of your answer. If you need to inspect contents, use the relevant file or directory tools.",
        message.trim_end(),
        attachment_block
    )
}
