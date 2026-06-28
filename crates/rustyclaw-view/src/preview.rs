//! Component data for the live preview pane (desktop only).
//!
//! Provides file-following and live rendering of HTML/Markdown/images.

/// Kind of content being previewed.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum PreviewKind {
    #[default]
    None,
    Markdown,
    Html,
    Image,
    Pdf,
    PlainText,
}

/// Display data for the preview pane.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PreviewPanelData {
    /// File currently being previewed.
    pub file_path: Option<String>,
    /// Content kind.
    pub kind: PreviewKind,
    /// Rendered content (HTML or plain text).
    pub content: String,
    /// Whether file-follow mode is active (auto-refresh on change).
    pub following: bool,
    /// Last modification timestamp.
    pub last_modified: Option<String>,
    /// Error message if preview failed.
    pub error: Option<String>,
}

impl PreviewPanelData {
    /// File name (basename) for the tab/title.
    pub fn file_name(&self) -> &str {
        self.file_path
            .as_deref()
            .and_then(|p| p.rsplit('/').next())
            .unwrap_or("(no file)")
    }

    /// Whether the preview pane is showing content.
    pub fn is_active(&self) -> bool {
        self.file_path.is_some() && self.error.is_none()
    }

    /// Kind label.
    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            PreviewKind::None => "None",
            PreviewKind::Markdown => "Markdown",
            PreviewKind::Html => "HTML",
            PreviewKind::Image => "Image",
            PreviewKind::Pdf => "PDF",
            PreviewKind::PlainText => "Text",
        }
    }
}
