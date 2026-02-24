// ── Ratatui TUI palette ─────────────────────────────────────────────────────
//
// Pre-built `ratatui::style::Color` and `Style` values derived from the
// lobster palette, for use in TUI pane rendering.
//
// Design inspiration: Crush/OpenCode TUI (charmbracelet/crush)
// - Left border highlights ("bubble stripes") for message focus
// - Rich 12-step grayscale surface system
// - Clear visual hierarchy through borders and spacing

use ratatui::style::{Color, Modifier, Style};

use rustyclaw_core::theme::palette;

// Convenience: convert palette tuple to ratatui Color.
const fn rgb(c: (u8, u8, u8)) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

// ── Border Characters (à la Crush/OpenCode) ─────────────────────

/// Thin left border for blurred/inactive items
pub const BORDER_THIN: &str = "│";
/// Thick left border for focused/active items
pub const BORDER_THICK: &str = "▌";
/// Section separator line
pub const SECTION_SEP: &str = "─";

// ── Status Icons ────────────────────────────────────────────────

/// Pending operation (tool call in progress)
pub const ICON_PENDING: &str = "●";
/// Success / completed
pub const ICON_SUCCESS: &str = "✓";
/// Error / failed
pub const ICON_ERROR: &str = "×";
/// Cancelled operation
pub const ICON_CANCELLED: &str = "○";

// ── Colours ─────────────────────────────────────────────

pub const ACCENT: Color = rgb(palette::ACCENT);
pub const ACCENT_BRIGHT: Color = rgb(palette::ACCENT_BRIGHT);
pub const ACCENT_DIM: Color = rgb(palette::ACCENT_DIM);
pub const INFO: Color = rgb(palette::INFO);
pub const SUCCESS: Color = rgb(palette::SUCCESS);
pub const WARN: Color = rgb(palette::WARN);
pub const ERROR: Color = rgb(palette::ERROR);
pub const MUTED: Color = rgb(palette::MUTED);

// Extra neutrals for the TUI - 12-step grayscale like Crush/OpenCode
pub const SURFACE_1: Color = Color::Rgb(0x0A, 0x0A, 0x0A); // deepest background
pub const SURFACE_2: Color = Color::Rgb(0x14, 0x14, 0x14); // panel background
pub const SURFACE_3: Color = Color::Rgb(0x1E, 0x1E, 0x1E); // element background
pub const SURFACE_4: Color = Color::Rgb(0x28, 0x28, 0x28); // hover/elevated
pub const SURFACE_5: Color = Color::Rgb(0x32, 0x32, 0x32); // active element
pub const SURFACE_6: Color = Color::Rgb(0x3C, 0x3C, 0x3C); // borders
pub const SURFACE_7: Color = Color::Rgb(0x48, 0x48, 0x48); // subtle border
pub const SURFACE_8: Color = Color::Rgb(0x60, 0x60, 0x60); // active border
pub const SURFACE_9: Color = Color::Rgb(0x82, 0x82, 0x82); // muted text
pub const SURFACE_10: Color = Color::Rgb(0xA0, 0xA0, 0xA0); // dim text
pub const SURFACE_11: Color = Color::Rgb(0xC0, 0xC0, 0xC0); // secondary text
pub const SURFACE_12: Color = Color::Rgb(0xEE, 0xEE, 0xEE); // primary text

// Backwards compatibility aliases
pub const SURFACE: Color = SURFACE_1;
pub const SURFACE_BRIGHT: Color = SURFACE_3;
pub const TEXT: Color = SURFACE_12;
pub const TEXT_DIM: Color = SURFACE_9;

// Message bubbles - more contrast between roles
pub const BG_USER: Color = Color::Rgb(0x1A, 0x1A, 0x1A); // dark for user input
pub const BG_ASSISTANT: Color = Color::Rgb(0x14, 0x14, 0x14); // slightly lighter for bot
pub const BG_CODE: Color = Color::Rgb(0x1E, 0x1E, 0x22); // subtle blue tint for code
pub const BG_THINKING: Color = Color::Rgb(0x18, 0x16, 0x14); // warm tint for thinking

// ── Pre-built styles ────────────────────────────────────

/// Border style for focused pane.
pub const fn focused_border() -> Style {
    Style::new().fg(ACCENT_BRIGHT)
}

/// Border style for unfocused pane.
pub const fn unfocused_border() -> Style {
    Style::new().fg(SURFACE_6)
}

/// Active/selected border.
pub const fn active_border() -> Style {
    Style::new().fg(ACCENT)
}

/// Subtle border for contained elements.
pub const fn subtle_border() -> Style {
    Style::new().fg(SURFACE_5)
}

/// Pane title when focused.
pub const fn title_focused() -> Style {
    Style::new().fg(ACCENT_BRIGHT).add_modifier(Modifier::BOLD)
}

/// Pane title when unfocused.
pub const fn title_unfocused() -> Style {
    Style::new().fg(SURFACE_10)
}

/// Style for the input prompt indicator when active.
pub const fn prompt_active() -> Style {
    Style::new().fg(ACCENT_BRIGHT).add_modifier(Modifier::BOLD)
}

/// Style for the input prompt indicator when inactive.
pub const fn prompt_inactive() -> Style {
    Style::new().fg(MUTED)
}

/// Status line hint text style.
pub const fn hint() -> Style {
    Style::new().fg(SURFACE_9)
}

/// Highlighted / selected item in a list.
pub const fn selected() -> Style {
    Style::new()
        .fg(SURFACE_12)
        .bg(SURFACE_4)
        .add_modifier(Modifier::BOLD)
}

/// Completion popup background.
pub const fn popup_bg() -> Style {
    Style::new().bg(SURFACE_3).fg(SURFACE_12)
}

/// Highlighted completion entry.
pub const fn popup_selected() -> Style {
    Style::new()
        .fg(ACCENT_BRIGHT)
        .bg(SURFACE_4)
        .add_modifier(Modifier::BOLD)
}

/// Normal completion entry.
pub const fn popup_item() -> Style {
    Style::new().fg(SURFACE_10)
}

/// Style for user-typed messages ("▶ something").
pub const fn user_message() -> Style {
    Style::new().fg(ACCENT_BRIGHT)
}

/// Style for system/info messages.
pub const fn system_message() -> Style {
    Style::new().fg(INFO)
}

/// Style for gateway response messages.
pub const fn gateway_message() -> Style {
    Style::new().fg(TEXT)
}

// ── Message Bubble Styles (Crush/OpenCode inspired) ────────────
//
// These styles use left border "stripes" to indicate message role
// and focus state. Use with the BORDER_THIN/BORDER_THICK chars.

/// Message bubble border colors by role
pub mod bubble {
    use super::*;

    /// User message border color (lobster accent)
    pub const USER_BORDER: Color = ACCENT_BRIGHT;
    /// Assistant message border color (success green for active)
    pub const ASSISTANT_BORDER: Color = SUCCESS;
    /// Tool call border color (muted)
    pub const TOOL_BORDER: Color = SURFACE_7;
    /// System/info border color
    pub const SYSTEM_BORDER: Color = INFO;
    /// Error border color
    pub const ERROR_BORDER: Color = ERROR;

    /// User message: focused state (thick border, bright accent)
    pub const fn user_focused() -> Style {
        Style::new().fg(USER_BORDER)
    }

    /// User message: blurred state (thin border, dimmer)
    pub const fn user_blurred() -> Style {
        Style::new().fg(ACCENT_DIM)
    }

    /// Assistant message: focused state (thick border, green)
    pub const fn assistant_focused() -> Style {
        Style::new().fg(ASSISTANT_BORDER)
    }

    /// Assistant message: blurred state (no border, just padding)
    pub const fn assistant_blurred() -> Style {
        Style::new().fg(SURFACE_6)
    }

    /// Tool call: focused state
    pub const fn tool_focused() -> Style {
        Style::new().fg(ASSISTANT_BORDER)
    }

    /// Tool call: blurred state
    pub const fn tool_blurred() -> Style {
        Style::new().fg(TOOL_BORDER)
    }

    /// Error message border
    pub const fn error() -> Style {
        Style::new().fg(ERROR_BORDER)
    }

    /// System message border
    pub const fn system() -> Style {
        Style::new().fg(SYSTEM_BORDER)
    }

    /// Thinking message border (dim, subtle)
    pub const fn thinking() -> Style {
        Style::new().fg(TOOL_BORDER)
    }
}

// ── Tool Status Styles ─────────────────────────────────────────

/// Pending tool operation (greenish, in progress)
pub const fn tool_pending() -> Style {
    Style::new().fg(Color::Rgb(0x2F, 0x8F, 0x51)) // darker green
}

/// Successful tool operation
pub const fn tool_success() -> Style {
    Style::new().fg(SUCCESS)
}

/// Failed tool operation
pub const fn tool_error() -> Style {
    Style::new().fg(ERROR)
}

/// Cancelled tool operation
pub const fn tool_cancelled() -> Style {
    Style::new().fg(MUTED)
}

// ── Diff colors ────────────────────────────────────────────────

pub const DIFF_ADDED: Color = Color::Rgb(0x4F, 0xD6, 0xBE);
pub const DIFF_REMOVED: Color = Color::Rgb(0xC5, 0x3B, 0x53);
pub const DIFF_CONTEXT: Color = Color::Rgb(0x82, 0x8B, 0xB8);

pub const DIFF_ADDED_BG: Color = Color::Rgb(0x20, 0x30, 0x3B);
pub const DIFF_REMOVED_BG: Color = Color::Rgb(0x37, 0x22, 0x2C);
pub const DIFF_CONTEXT_BG: Color = SURFACE_2;

// ── Markdown StyleSheet for tui-markdown ────────────────────

/// Custom stylesheet for markdown rendering that uses the lobster palette.
#[derive(Debug, Clone, Copy, Default)]
pub struct RustyClawMarkdownStyle;

impl tui_markdown::StyleSheet for RustyClawMarkdownStyle {
    fn heading(&self, level: u8) -> Style {
        match level {
            1 => Style::new()
                .fg(ACCENT)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            2 => Style::new().fg(ACCENT_BRIGHT).add_modifier(Modifier::BOLD),
            3 => Style::new()
                .fg(ACCENT_DIM)
                .add_modifier(Modifier::BOLD | Modifier::ITALIC),
            _ => Style::new().fg(INFO).add_modifier(Modifier::ITALIC),
        }
    }

    fn code(&self) -> Style {
        Style::new().fg(TEXT).bg(BG_CODE)
    }

    fn link(&self) -> Style {
        Style::new().fg(INFO).add_modifier(Modifier::UNDERLINED)
    }

    fn blockquote(&self) -> Style {
        Style::new().fg(MUTED).add_modifier(Modifier::ITALIC)
    }

    fn heading_meta(&self) -> Style {
        Style::new().fg(TEXT_DIM).add_modifier(Modifier::DIM)
    }

    fn metadata_block(&self) -> Style {
        Style::new().fg(WARN)
    }
}
