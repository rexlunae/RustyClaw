//! Component data for rich media rendering in tool results.

/// The kind of media attached to a tool result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Audio,
    Pdf,
    Html,
    Canvas,
}

/// Data for a media attachment on a tool result, ready for rendering.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaData {
    pub kind: MediaKind,
    /// Path to the file on the agent filesystem (for desktop file:// links).
    pub path: Option<String>,
    /// MIME type.
    pub mime: Option<String>,
    /// Whether inline data bytes are available.
    pub has_data: bool,
}

impl MediaData {
    /// Convert from the protocol DTO.
    pub fn from_payload(p: &rustyclaw_core::gateway::protocol::frames::MediaPayload) -> Self {
        use rustyclaw_core::gateway::protocol::frames::MediaKind as PK;
        let kind = match p.kind {
            PK::Image => MediaKind::Image,
            PK::Audio => MediaKind::Audio,
            PK::Pdf => MediaKind::Pdf,
            PK::Html => MediaKind::Html,
            PK::Canvas => MediaKind::Canvas,
        };
        Self {
            kind,
            path: p.path.clone(),
            mime: p.mime.clone(),
            has_data: p.data.is_some(),
        }
    }

    /// Icon for this media type.
    pub fn icon(&self) -> &'static str {
        match self.kind {
            MediaKind::Image => "🖼️",
            MediaKind::Audio => "🔊",
            MediaKind::Pdf => "📄",
            MediaKind::Html => "🌐",
            MediaKind::Canvas => "🎨",
        }
    }

    /// Human label for this media type.
    pub fn label(&self) -> &'static str {
        match self.kind {
            MediaKind::Image => "Image",
            MediaKind::Audio => "Audio",
            MediaKind::Pdf => "PDF",
            MediaKind::Html => "HTML",
            MediaKind::Canvas => "Canvas",
        }
    }

    /// File extension hint derived from MIME or kind.
    pub fn extension_hint(&self) -> &str {
        if let Some(ref mime) = self.mime {
            match mime.as_str() {
                "image/png" => "png",
                "image/jpeg" | "image/jpg" => "jpg",
                "image/webp" => "webp",
                "image/gif" => "gif",
                "audio/wav" => "wav",
                "audio/mp3" | "audio/mpeg" => "mp3",
                "audio/ogg" => "ogg",
                "application/pdf" => "pdf",
                _ => "",
            }
        } else {
            match self.kind {
                MediaKind::Image => "png",
                MediaKind::Audio => "wav",
                MediaKind::Pdf => "pdf",
                MediaKind::Html => "html",
                MediaKind::Canvas => "html",
            }
        }
    }

    /// TUI fallback display (no inline rendering capability).
    pub fn tui_fallback(&self) -> String {
        let icon = self.icon();
        let label = self.label();
        if let Some(ref path) = self.path {
            format!("{icon} [{label}] {path}")
        } else {
            format!("{icon} [{label}] (inline)")
        }
    }
}
