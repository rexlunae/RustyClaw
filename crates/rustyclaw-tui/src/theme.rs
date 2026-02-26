// ── RustyClaw TUI Theme ─────────────────────────────────────────────────────
//
// Colour palette for the iocraft TUI.
// Follows the "lobster palette" from rustyclaw-core/src/theme.rs
//
// | Token          | Hex       | Usage                          |
// |----------------|-----------|--------------------------------|
// | accent         | `#FF5A2D` | headings, labels, primary      |
// | accent_bright  | `#FF7A3D` | command names, emphasis        |
// | accent_dim     | `#D14A22` | secondary highlight            |
// | info           | `#FF8A5B` | informational values           |
// | success        | `#2FBF71` | success states                 |
// | warn           | `#FFB020` | warnings, fallbacks            |
// | error          | `#E23D2D` | errors, failures               |
// | muted          | `#8B7F77` | de-emphasis, metadata          |

use iocraft::prelude::*;
use rustyclaw_core::types::MessageRole;

// ── Accent (lobster orange) ─────────────────────────────────────────────────

pub const ACCENT: Color = Color::Rgb {
    r: 0xFF,
    g: 0x5A,
    b: 0x2D,
};
pub const ACCENT_BRIGHT: Color = Color::Rgb {
    r: 0xFF,
    g: 0x7A,
    b: 0x3D,
};
pub const ACCENT_DIM: Color = Color::Rgb {
    r: 0xD1,
    g: 0x4A,
    b: 0x22,
};

// ── Text ────────────────────────────────────────────────────────────────────

pub const TEXT: Color = Color::Rgb {
    r: 0xE8,
    g: 0xE0,
    b: 0xD8,
};
pub const TEXT_DIM: Color = Color::Rgb {
    r: 0xA0,
    g: 0x98,
    b: 0x90,
};
pub const MUTED: Color = Color::Rgb {
    r: 0x8B,
    g: 0x7F,
    b: 0x77,
};

// ── Semantic ────────────────────────────────────────────────────────────────

pub const INFO: Color = Color::Rgb {
    r: 0xFF,
    g: 0x8A,
    b: 0x5B,
};
pub const SUCCESS: Color = Color::Rgb {
    r: 0x2F,
    g: 0xBF,
    b: 0x71,
};
pub const WARN: Color = Color::Rgb {
    r: 0xFF,
    g: 0xB0,
    b: 0x20,
};
pub const ERROR: Color = Color::Rgb {
    r: 0xE2,
    g: 0x3D,
    b: 0x2D,
};

// ── Backgrounds ─────────────────────────────────────────────────────────────

pub const BG_MAIN: Color = Color::Rgb {
    r: 0x1A,
    g: 0x18,
    b: 0x16,
};
pub const BG_SURFACE: Color = Color::Rgb {
    r: 0x24,
    g: 0x20,
    b: 0x1C,
};
pub const BG_USER: Color = Color::Rgb {
    r: 0x2A,
    g: 0x22,
    b: 0x1A,
};
pub const BG_ASSISTANT: Color = Color::Rgb {
    r: 0x22,
    g: 0x1E,
    b: 0x1A,
};
pub const BG_CODE: Color = Color::Rgb {
    r: 0x1E,
    g: 0x1A,
    b: 0x16,
};

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
        MessageRole::Assistant => ACCENT_DIM,
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
