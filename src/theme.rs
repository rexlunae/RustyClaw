//! Terminal theme & spinner helpers.
//!
//! Mirrors openclaw's "lobster palette" (`src/terminal/palette.ts`) and
//! `src/terminal/theme.ts`.  Respects the `NO_COLOR` env-var and the
//! `--no-color` CLI flag.
//!
//! # Palette (from openclaw docs/cli/index.md)
//!
//! | Token          | Hex       | Usage                          |
//! |----------------|-----------|--------------------------------|
//! | accent         | `#FF5A2D` | headings, labels, primary      |
//! | accent_bright  | `#FF7A3D` | command names, emphasis         |
//! | accent_dim     | `#D14A22` | secondary highlight             |
//! | info           | `#FF8A5B` | informational values            |
//! | success        | `#2FBF71` | success states                  |
//! | warn           | `#FFB020` | warnings, fallbacks             |
//! | error          | `#E23D2D` | errors, failures                |
//! | muted          | `#8B7F77` | de-emphasis, metadata           |

use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

// ── Global color toggle ─────────────────────────────────────────────────────

static COLOR_DISABLED: AtomicBool = AtomicBool::new(false);

/// Call once at startup (after CLI parsing) to disable colour globally.
pub fn disable_color() {
    COLOR_DISABLED.store(true, Ordering::Relaxed);
    colored::control::set_override(false);
}

/// Initialise the colour system.  Checks `NO_COLOR` env-var and optional
/// `--no-color` flag.
pub fn init_color(no_color_flag: bool) {
    if no_color_flag
        || std::env::var("NO_COLOR")
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    {
        disable_color();
    }
}

fn is_color() -> bool {
    !COLOR_DISABLED.load(Ordering::Relaxed)
}

// ── Lobster palette ─────────────────────────────────────────────────────────

/// Lobster palette hex values — source of truth.
pub mod palette {
    pub const ACCENT: (u8, u8, u8) = (0xFF, 0x5A, 0x2D);
    pub const ACCENT_BRIGHT: (u8, u8, u8) = (0xFF, 0x7A, 0x3D);
    pub const ACCENT_DIM: (u8, u8, u8) = (0xD1, 0x4A, 0x22);
    pub const INFO: (u8, u8, u8) = (0xFF, 0x8A, 0x5B);
    pub const SUCCESS: (u8, u8, u8) = (0x2F, 0xBF, 0x71);
    pub const WARN: (u8, u8, u8) = (0xFF, 0xB0, 0x20);
    pub const ERROR: (u8, u8, u8) = (0xE2, 0x3D, 0x2D);
    pub const MUTED: (u8, u8, u8) = (0x8B, 0x7F, 0x77);
}

// ── Themed formatting helpers ───────────────────────────────────────────────
//
// Each function returns a `String` so callers can `println!("{}", accent("…"))`.

fn apply(text: &str, rgb: (u8, u8, u8)) -> String {
    if is_color() {
        text.truecolor(rgb.0, rgb.1, rgb.2).to_string()
    } else {
        text.to_string()
    }
}

fn apply_bold(text: &str, rgb: (u8, u8, u8)) -> String {
    if is_color() {
        text.truecolor(rgb.0, rgb.1, rgb.2).bold().to_string()
    } else {
        text.to_string()
    }
}

/// Primary accent (headings, labels).
pub fn accent(text: &str) -> String {
    apply(text, palette::ACCENT)
}

/// Bright accent (command names, emphasis).
pub fn accent_bright(text: &str) -> String {
    apply(text, palette::ACCENT_BRIGHT)
}

/// Dim accent (secondary highlight).
pub fn accent_dim(text: &str) -> String {
    apply(text, palette::ACCENT_DIM)
}

/// Informational values.
pub fn info(text: &str) -> String {
    apply(text, palette::INFO)
}

/// Success state.
pub fn success(text: &str) -> String {
    apply(text, palette::SUCCESS)
}

/// Warning / attention.
pub fn warn(text: &str) -> String {
    apply(text, palette::WARN)
}

/// Error / failure.
pub fn error(text: &str) -> String {
    apply(text, palette::ERROR)
}

/// De-emphasis / metadata.
pub fn muted(text: &str) -> String {
    apply(text, palette::MUTED)
}

/// Bold heading in accent colour.
pub fn heading(text: &str) -> String {
    apply_bold(text, palette::ACCENT)
}

/// Bold text (no colour).
pub fn bold(text: &str) -> String {
    if is_color() {
        text.bold().to_string()
    } else {
        text.to_string()
    }
}

/// Dimmed text (terminal dim attribute).
pub fn dim(text: &str) -> String {
    if is_color() {
        text.dimmed().to_string()
    } else {
        text.to_string()
    }
}

// ── Composite icons ─────────────────────────────────────────────────────────
//
// openclaw uses ✓ / ✗ / ⚠ with colour.

/// Green ✓
pub fn icon_ok(label: &str) -> String {
    format!("{} {}", success("✓"), label)
}

/// Red ✗
pub fn icon_fail(label: &str) -> String {
    format!("{} {}", error("✗"), label)
}

/// Yellow ⚠
pub fn icon_warn(label: &str) -> String {
    format!("{} {}", warn("⚠"), label)
}

/// Muted dash —
pub fn icon_muted(label: &str) -> String {
    format!("{} {}", muted("·"), muted(label))
}

// ── Labelled key : value ────────────────────────────────────────────────────

/// Format "  Label  : value" with the label dimmed and the value in accent.
pub fn label_value(label: &str, value: &str) -> String {
    format!("  {} : {}", muted(label), info(value))
}

// ── Spinner helpers ─────────────────────────────────────────────────────────

/// Spinner character set mimicking openclaw's clack spinners.
const SPINNER_CHARS: &[&str] = &["◒", "◐", "◓", "◑"];

/// Create an indeterminate spinner with a message.
///
/// Returns a `ProgressBar` that the caller should call `.finish_with_message()`
/// or `.finish_and_clear()` on when done.
pub fn spinner(message: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    let style = if is_color() {
        ProgressStyle::with_template(&format!(
            "{{spinner:.{}}}  {{msg}}",
            "red" // indicatif colour name closest to lobster accent
        ))
        .unwrap()
        .tick_strings(SPINNER_CHARS)
    } else {
        ProgressStyle::with_template("{spinner}  {msg}")
            .unwrap()
            .tick_strings(SPINNER_CHARS)
    };
    pb.set_style(style);
    pb.set_message(message.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

/// Finish a spinner with a success icon + message.
pub fn spinner_ok(pb: &ProgressBar, message: &str) {
    pb.finish_with_message(icon_ok(message));
}

/// Finish a spinner with a failure icon + message.
pub fn spinner_fail(pb: &ProgressBar, message: &str) {
    pb.finish_with_message(icon_fail(message));
}

/// Finish a spinner with a warning icon + message.
pub fn spinner_warn(pb: &ProgressBar, message: &str) {
    pb.finish_with_message(icon_warn(message));
}

// ── Box drawing (for onboarding banner etc.) ────────────────────────────────

/// Print a styled header box (like openclaw's `intro()` from clack).
pub fn print_header(title: &str) {
    use unicode_width::UnicodeWidthStr;

    let display_w = UnicodeWidthStr::width(title);
    // Inner width = display width of title + at least 4 chars padding (2 each side)
    let inner = (display_w + 4).max(42);
    let pad = inner - display_w;
    let left = pad / 2;
    let right = pad - left;
    println!();
    println!("{}", accent(&format!("┌{}┐", "─".repeat(inner))));
    println!(
        "{}",
        accent(&format!(
            "│{}{}{}│",
            " ".repeat(left),
            title,
            " ".repeat(right)
        ))
    );
    println!("{}", accent(&format!("└{}┘", "─".repeat(inner))));
    println!();
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_color_output() {
        // Force no-color mode (both our flag AND the colored crate).
        COLOR_DISABLED.store(true, Ordering::Relaxed);
        colored::control::set_override(false);
        assert_eq!(accent("hello"), "hello");
        assert_eq!(success("ok"), "ok");
        assert_eq!(error("fail"), "fail");
        assert_eq!(icon_ok("done"), "✓ done");
        assert_eq!(icon_fail("bad"), "✗ bad");
        // Reset for other tests.
        colored::control::unset_override();
        COLOR_DISABLED.store(false, Ordering::Relaxed);
    }

    #[test]
    fn test_label_value() {
        COLOR_DISABLED.store(true, Ordering::Relaxed);
        let out = label_value("Key", "/some/path");
        assert!(out.contains("Key"));
        assert!(out.contains("/some/path"));
        COLOR_DISABLED.store(false, Ordering::Relaxed);
    }
}

// ── Ratatui palette ─────────────────────────────────────────────────────────
//
// Pre-built `ratatui::style::Color` and `Style` values derived from the
// lobster palette, for use in TUI pane rendering.

#[cfg(feature = "tui")]
pub mod tui_palette {
    use ratatui::style::{Color, Modifier, Style};

    use super::palette;

    // Convenience: convert palette tuple to ratatui Color.
    const fn rgb(c: (u8, u8, u8)) -> Color {
        Color::Rgb(c.0, c.1, c.2)
    }

    // ── Colours ─────────────────────────────────────────────

    pub const ACCENT: Color = rgb(palette::ACCENT);
    pub const ACCENT_BRIGHT: Color = rgb(palette::ACCENT_BRIGHT);
    pub const ACCENT_DIM: Color = rgb(palette::ACCENT_DIM);
    pub const INFO: Color = rgb(palette::INFO);
    pub const SUCCESS: Color = rgb(palette::SUCCESS);
    pub const WARN: Color = rgb(palette::WARN);
    pub const ERROR: Color = rgb(palette::ERROR);
    pub const MUTED: Color = rgb(palette::MUTED);

    // Extra neutrals for the TUI - 12-step grayscale like opencode
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
}
