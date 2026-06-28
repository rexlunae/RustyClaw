//! Component data for voice input/output (STT + TTS).
//!
//! Gated behind the `voice` feature in consumer crates.

/// Voice recording state.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum VoiceState {
    #[default]
    Idle,
    Listening,
    Processing,
    Speaking,
    Error(String),
}

/// Display data for the voice I/O panel.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct VoiceData {
    pub state: VoiceState,
    /// Current transcription (partial or final).
    pub transcript: String,
    /// Whether auto-send is enabled (send on silence).
    pub auto_send: bool,
    /// Volume level for visual feedback (0.0 - 1.0).
    pub volume: f32,
    /// TTS voice name.
    pub voice: String,
    /// Whether TTS is muted.
    pub muted: bool,
}

impl VoiceData {
    /// Human-friendly state label.
    pub fn state_label(&self) -> &str {
        match &self.state {
            VoiceState::Idle => "Ready",
            VoiceState::Listening => "Listening...",
            VoiceState::Processing => "Processing...",
            VoiceState::Speaking => "Speaking...",
            VoiceState::Error(e) => e.as_str(),
        }
    }

    /// Icon for the current state.
    pub fn state_icon(&self) -> &'static str {
        match &self.state {
            VoiceState::Idle => "🎙️",
            VoiceState::Listening => "🔴",
            VoiceState::Processing => "⏳",
            VoiceState::Speaking => "🔊",
            VoiceState::Error(_) => "⚠️",
        }
    }

    /// Whether the mic button should pulse (recording or processing).
    pub fn is_active(&self) -> bool {
        matches!(self.state, VoiceState::Listening | VoiceState::Processing)
    }

    /// Volume bar display (for TUI, renders N-wide bar).
    pub fn volume_bar(&self, width: usize) -> String {
        let filled = (self.volume * width as f32).round() as usize;
        let empty = width.saturating_sub(filled);
        format!("{}{}", "█".repeat(filled), "░".repeat(empty))
    }
}
