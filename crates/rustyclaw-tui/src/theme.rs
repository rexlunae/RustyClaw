// ── RustyClaw TUI Theme ─────────────────────────────────────────────────────
//
// Colour palette for the iocraft TUI.

use iocraft::prelude::*;
use rustyclaw_core::types::MessageRole;

// ── Accent (teal / cyan) ────────────────────────────────────────────────────

pub const ACCENT: Color = Color::Rgb { r: 0, g: 175, b: 175 };
pub const ACCENT_BRIGHT: Color = Color::Rgb { r: 0, g: 215, b: 215 };
pub const ACCENT_DIM: Color = Color::Rgb { r: 0, g: 95, b: 95 };

// ── Text ────────────────────────────────────────────────────────────────────

pub const TEXT: Color = Color::Rgb { r: 198, g: 208, b: 220 };
pub const TEXT_DIM: Color = Color::Rgb { r: 110, g: 120, b: 135 };
pub const MUTED: Color = Color::Rgb { r: 75, g: 85, b: 99 };

// ── Semantic ────────────────────────────────────────────────────────────────

pub const INFO: Color = Color::Rgb { r: 66, g: 165, b: 245 };
pub const SUCCESS: Color = Color::Rgb { r: 102, g: 187, b: 106 };
pub const WARN: Color = Color::Rgb { r: 255, g: 167, b: 38 };
pub const ERROR: Color = Color::Rgb { r: 239, g: 83, b: 80 };

// ── Backgrounds ─────────────────────────────────────────────────────────────

pub const BG_MAIN: Color = Color::Rgb { r: 22, g: 22, b: 30 };
pub const BG_SURFACE: Color = Color::Rgb { r: 30, g: 30, b: 40 };
pub const BG_USER: Color = Color::Rgb { r: 24, g: 35, b: 45 };
pub const BG_ASSISTANT: Color = Color::Rgb { r: 28, g: 28, b: 38 };
pub const BG_CODE: Color = Color::Rgb { r: 26, g: 26, b: 36 };

// ── Spinner frames ──────────────────────────────────────────────────────────

pub const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

// ── Role helpers ────────────────────────────────────────────────────────────

pub fn role_color(role: &MessageRole) -> Color {
    match role {
        MessageRole::User => ACCENT_BRIGHT,
        MessageRole::Assistant => TEXT,
        MessageRole::Info => INFO,
        MessageRole::Success => SUCCESS,
        MessageRole::Warning => WARN,
        MessageRole::Error => ERROR,
        MessageRole::System => MUTED,
        MessageRole::ToolCall => MUTED,
        MessageRole::ToolResult => TEXT_DIM,
        MessageRole::Thinking => MUTED,
    }
}

pub fn role_bg(role: &MessageRole) -> Color {
    match role {
        MessageRole::User => BG_USER,
        MessageRole::Assistant => BG_ASSISTANT,
        MessageRole::ToolCall | MessageRole::ToolResult => BG_CODE,
        _ => BG_SURFACE,
    }
}

pub fn role_border(role: &MessageRole) -> Color {
    match role {
        MessageRole::User => ACCENT_BRIGHT,
        MessageRole::Assistant => MUTED,
        MessageRole::Error => ERROR,
        MessageRole::Warning => WARN,
        MessageRole::Success => SUCCESS,
        MessageRole::Info => INFO,
        _ => MUTED,
    }
}

pub fn gateway_color(status: &rustyclaw_core::types::GatewayStatus) -> Color {
    use rustyclaw_core::types::GatewayStatus::*;
    match status {
        Connected | ModelReady => SUCCESS,
        Connecting => WARN,
        Disconnected | Error | ModelError => ERROR,
        Unconfigured => MUTED,
        VaultLocked | AuthRequired => WARN,
    }
}

pub fn gateway_icon(status: &rustyclaw_core::types::GatewayStatus) -> &'static str {
    use rustyclaw_core::types::GatewayStatus::*;
    match status {
        Connected | ModelReady => "●",
        Connecting => "◌",
        Disconnected => "○",
        Error | ModelError => "✖",
        Unconfigured => "○",
        VaultLocked => "🔒",
        AuthRequired => "🔑",
    }
}
