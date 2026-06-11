//! Semantic tone — framework-agnostic colour semantics shared by clients.
//!
//! A [`Tone`] describes *what kind* of thing is being shown (success,
//! danger, …) without committing to a colour system.  The desktop client
//! maps tones to Bulma colour modifiers; the TUI maps them to its
//! terminal palette.  Centralising the mapping here keeps "which colour
//! does a failed tool call get" decisions out of the renderers.

/// Semantic colour intent for chips, banners, badges, and buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tone {
    /// No specific intent — rendered in the neutral scheme colour.
    #[default]
    Neutral,
    /// Brand-primary emphasis.
    Primary,
    /// Informational (in-progress, FYI).
    Info,
    /// Positive outcome (connected, done).
    Success,
    /// Needs attention but not fatal.
    Warning,
    /// Error / destructive.
    Danger,
}

impl Tone {
    /// Bulma-style CSS modifier class (`"is-success"`, …).
    ///
    /// Neutral has no modifier and returns the empty string.
    pub fn css_class(self) -> &'static str {
        match self {
            Tone::Neutral => "",
            Tone::Primary => "is-primary",
            Tone::Info => "is-info",
            Tone::Success => "is-success",
            Tone::Warning => "is-warning",
            Tone::Danger => "is-danger",
        }
    }
}
