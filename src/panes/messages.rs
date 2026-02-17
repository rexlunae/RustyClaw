use std::sync::atomic::AtomicU16;

use anyhow::Result;
use ratatui::{
    layout::{Constraint, Rect},
    prelude::*,
    widgets::{Paragraph, Wrap},
};

use crate::action::Action;
use crate::panes::{DisplayMessage, MessageRole, Pane, PaneState};
use crate::theme::tui_palette as tp;
use crate::tui::Frame;

// ── Global tab-width setting (read by get_lines) ──────────────────────

static TAB_WIDTH: AtomicU16 = AtomicU16::new(5);

/// Width of the left border stripe (1 char + 1 space padding)
const BORDER_WIDTH: u16 = 2;

pub struct MessagesPane {
    focused: bool,
    /// Index of the currently focused message (for selection/navigation)
    focused_message: Option<usize>,
    /// Vertical scroll offset in visual (wrapped) lines from the bottom.
    /// `usize::MAX` = pinned to the newest content (auto-scroll).
    scroll_offset: usize,
}

impl MessagesPane {
    pub fn new(focused: bool, _focused_border_style: Style) -> Self {
        Self {
            focused,
            focused_message: None,
            scroll_offset: usize::MAX,
        }
    }

    /// Map a [`MessageRole`] to its foreground colour.
    #[allow(dead_code)]
    fn role_color(role: &MessageRole) -> Color {
        match role {
            MessageRole::User => tp::ACCENT_BRIGHT,
            MessageRole::Assistant => tp::TEXT,
            MessageRole::Info => tp::INFO,
            MessageRole::Success => tp::SUCCESS,
            MessageRole::Warning => tp::WARN,
            MessageRole::Error => tp::ERROR,
            MessageRole::System => tp::MUTED,
            MessageRole::ToolCall => tp::MUTED,
            MessageRole::ToolResult => tp::TEXT_DIM,
        }
    }

    /// Map a [`MessageRole`] to an optional subtle background colour.
    fn role_bg(role: &MessageRole) -> Option<Color> {
        match role {
            MessageRole::User => Some(tp::BG_USER),
            MessageRole::Assistant => Some(tp::BG_ASSISTANT),
            MessageRole::ToolCall => Some(tp::BG_CODE),
            MessageRole::ToolResult => Some(tp::BG_CODE),
            _ => None,
        }
    }

    /// Get the left border character and style for a message role.
    /// Returns (border_char, border_style, use_border).
    fn role_border(role: &MessageRole, is_focused: bool) -> (&'static str, Style, bool) {
        match role {
            MessageRole::User => {
                if is_focused {
                    (tp::BORDER_THICK, tp::bubble::user_focused(), true)
                } else {
                    (tp::BORDER_THIN, tp::bubble::user_blurred(), true)
                }
            }
            MessageRole::Assistant => {
                if is_focused {
                    (tp::BORDER_THICK, tp::bubble::assistant_focused(), true)
                } else {
                    // No border when blurred, just padding
                    ("", tp::bubble::assistant_blurred(), false)
                }
            }
            MessageRole::ToolCall | MessageRole::ToolResult => {
                if is_focused {
                    (tp::BORDER_THICK, tp::bubble::tool_focused(), true)
                } else {
                    (tp::BORDER_THIN, tp::bubble::tool_blurred(), true)
                }
            }
            MessageRole::Error => {
                (tp::BORDER_THICK, tp::bubble::error(), true)
            }
            MessageRole::System | MessageRole::Info | MessageRole::Success | MessageRole::Warning => {
                (tp::BORDER_THIN, tp::bubble::system(), true)
            }
        }
    }

    /// Whether this role should show a leading icon.
    ///
    /// User and Assistant rely on background colour instead of an icon.
    #[allow(dead_code)]
    fn should_show_icon(role: &MessageRole) -> bool {
        !matches!(role, MessageRole::User | MessageRole::Assistant)
    }

    /// Copy text to the system clipboard using platform-native tools.
    fn copy_to_clipboard(text: &str) -> Result<()> {
        use anyhow::Context;
        use std::io::Write;
        use std::process::{Command, Stdio};

        #[cfg(target_os = "macos")]
        let mut child = Command::new("pbcopy")
            .stdin(Stdio::piped())
            .spawn()
            .context("Failed to launch pbcopy")?;

        #[cfg(target_os = "linux")]
        let mut child = {
            Command::new("xclip")
                .args(["-selection", "clipboard"])
                .stdin(Stdio::piped())
                .spawn()
                .or_else(|_| {
                    Command::new("xsel")
                        .arg("--clipboard")
                        .stdin(Stdio::piped())
                        .spawn()
                })
                .context("Failed to launch xclip or xsel")?
        };

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        anyhow::bail!("Clipboard not supported on this platform");

        if let Some(stdin) = child.stdin.as_mut() {
            stdin.write_all(text.as_bytes())?;
        }
        child.wait()?;
        Ok(())
    }

    // ── Layout helpers ──────────────────────────────────────────────────

    /// Get styled lines for a message (uses cache).
    fn get_lines(msg: &DisplayMessage) -> &Vec<Line<'static>> {
        let tab_stop = TAB_WIDTH.load(std::sync::atomic::Ordering::Relaxed) as usize;
        msg.get_lines(tab_stop)
    }

    /// Count how many visual (wrapped) rows a set of `Line`s occupies at `width`.
    fn visual_lines_count(lines: &[Line<'_>], width: u16) -> usize {
        if width == 0 {
            return lines.len().max(1);
        }
        let w = width as usize;
        lines
            .iter()
            .map(|line| {
                let text_width = line.width();
                if text_width == 0 {
                    1
                } else {
                    text_width.div_ceil(w)
                }
            })
            .sum::<usize>()
            .max(1)
    }

    /// Resolve the logical message index that the current visual scroll
    /// row falls within (used for the copy command).
    fn message_index_at_visual_row(
        visual_row: usize,
        messages: &[DisplayMessage],
        width: u16,
        spacing: u16,
    ) -> usize {
        let mut accum = 0usize;
        for (i, msg) in messages.iter().enumerate() {
            if i > 0 {
                accum += spacing as usize;
            }
            let lines = Self::get_lines(msg);
            let h = Self::visual_lines_count(lines, width);
            if accum + h > visual_row {
                return i;
            }
            accum += h;
        }
        messages.len().saturating_sub(1)
    }
}

impl Pane for MessagesPane {
    fn height_constraint(&self) -> Constraint {
        Constraint::Fill(3)
    }

    fn update(&mut self, action: Action, state: &mut PaneState<'_>) -> Result<Option<Action>> {
        match action {
            Action::Focus => {
                self.focused = true;
                let status = "[j/k → scroll] [c → copy] [/help → commands]";
                return Ok(Some(Action::TimedStatusLine(status.into(), 3)));
            }
            Action::UnFocus => {
                self.focused = false;
            }
            Action::Down => {
                if self.scroll_offset == usize::MAX {
                    // Already at bottom — nowhere to go.
                } else {
                    self.scroll_offset = self.scroll_offset.saturating_sub(1);
                }
            }
            Action::Up => {
                if self.scroll_offset == usize::MAX {
                    self.scroll_offset = 0;
                }
                self.scroll_offset = self.scroll_offset.saturating_add(1);
            }
            Action::Update => {
                // Auto-scroll to bottom on new content
                self.scroll_offset = usize::MAX;
            }
            Action::Tick => {
                // Keep pinned to bottom while loading
                if state.loading_line.is_some() && self.scroll_offset == usize::MAX {
                    // Already pinned — nothing to do.
                }
            }
            Action::CopyMessage => {
                // Map current scroll position back to a message index.
                let spacing = state.config.message_spacing;
                let msg_count = state.messages.len();
                let total: usize = state
                    .messages
                    .iter()
                    .map(|m| Self::visual_lines_count(Self::get_lines(m), 200))
                    .sum::<usize>()
                    + if msg_count > 1 {
                        (msg_count - 1) * spacing as usize
                    } else {
                        0
                    };
                let scroll_top = total.saturating_sub(
                    if self.scroll_offset == usize::MAX {
                        0
                    } else {
                        self.scroll_offset
                    },
                );
                let idx =
                    Self::message_index_at_visual_row(scroll_top, state.messages, 200, spacing);
                if let Some(msg) = state.messages.get(idx) {
                    match Self::copy_to_clipboard(&msg.content) {
                        Ok(()) => {
                            return Ok(Some(Action::TimedStatusLine(
                                "Copied to clipboard ✓".into(),
                                2,
                            )));
                        }
                        Err(e) => {
                            return Ok(Some(Action::TimedStatusLine(
                                format!("Copy failed: {}", e),
                                3,
                            )));
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn draw(&mut self, frame: &mut Frame<'_>, area: Rect, state: &PaneState<'_>) -> Result<()> {
        let width = area.width;
        if width == 0 || area.height == 0 {
            return Ok(());
        }

        // Debug: log message count on each draw
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/rustyclaw-tui.log")
        {
            use std::io::Write;
            let last_content_len = state.messages.last().map(|m| m.content.len()).unwrap_or(0);
            let _ = writeln!(
                file,
                "[{}] draw: messages={}, last_content_len={}, loading={:?}",
                chrono::Utc::now().format("%H:%M:%S%.3f"),
                state.messages.len(),
                last_content_len,
                state.loading_line.is_some()
            );
        }

        // Sync the tab-width setting so build_lines can read it.
        TAB_WIDTH.store(state.config.tab_width, std::sync::atomic::Ordering::Relaxed);

        // ── Build entries with pre-computed visual heights ───────────

        struct Entry<'a> {
            /// All rendered lines for this entry (may be many for multi-line messages).
            text: Text<'a>,
            bg: Option<Color>,
            /// Left border character (empty string = no border, just padding)
            border_char: &'static str,
            /// Style for the left border
            border_style: Style,
            /// Whether to show the border (false = just padding)
            show_border: bool,
            /// Total visual rows after wrapping.
            height: u16,
            /// Original message index (None for spacing/loading entries)
            msg_index: Option<usize>,
        }

        let spacing = state.config.message_spacing;
        // Account for border width when calculating text width
        let text_width = width.saturating_sub(BORDER_WIDTH);

        let mut entries: Vec<Entry<'_>> = Vec::new();
        for (i, msg) in state.messages.iter().enumerate() {
            // Insert blank spacing line(s) between messages
            if i > 0 && spacing > 0 {
                entries.push(Entry {
                    text: Text::from(""),
                    bg: None,
                    border_char: "",
                    border_style: Style::default(),
                    show_border: false,
                    height: spacing,
                    msg_index: None,
                });
            }
            let lines = Self::get_lines(msg);
            // Use text_width for wrapping calculation to account for border
            let h = Self::visual_lines_count(lines, text_width) as u16;
            
            // Determine if this message is focused
            let is_focused = self.focused_message == Some(i);
            let (border_char, border_style, show_border) = Self::role_border(&msg.role, is_focused);
            
            entries.push(Entry {
                text: Text::from(lines.clone()),
                bg: Self::role_bg(&msg.role),
                border_char,
                border_style,
                show_border,
                height: h,
                msg_index: Some(i),
            });
        }

        // Append loading line if active
        if let Some(ref loading) = state.loading_line {
            let line = Line::from(Span::styled(
                format!(" {}", loading),
                Style::default().fg(tp::ACCENT_BRIGHT),
            ));
            let h = Self::visual_lines_count(&[line.clone()], text_width) as u16;
            entries.push(Entry {
                text: Text::from(line),
                bg: None,
                border_char: tp::BORDER_THIN,
                border_style: tp::tool_pending(),
                show_border: true,
                height: h,
                msg_index: None,
            });
        }

        let total_visual: usize = entries.iter().map(|e| e.height as usize).sum();
        let viewport = area.height as usize;

        // ── Resolve scroll position ─────────────────────────────────
        let max_scroll = total_visual.saturating_sub(viewport);

        let from_bottom = if self.scroll_offset == usize::MAX {
            0
        } else {
            self.scroll_offset.min(max_scroll)
        };
        if self.scroll_offset != usize::MAX {
            self.scroll_offset = from_bottom;
        }

        let scroll_top = max_scroll - from_bottom;

        // ── Determine which entries are visible ─────────────────────

        let mut skipped: usize = 0;
        let mut render_start: usize = 0;
        let mut first_skip_rows: u16 = 0;

        for (i, entry) in entries.iter().enumerate() {
            let h = entry.height as usize;
            if skipped + h <= scroll_top {
                skipped += h;
                render_start = i + 1;
            } else {
                first_skip_rows = (scroll_top - skipped) as u16;
                render_start = i;
                break;
            }
        }

        // ── Render visible entries ──────────────────────────────────

        let mut y = area.y;
        let mut remaining = area.height;

        for (idx, entry) in entries.iter().enumerate() {
            if idx < render_start || remaining == 0 {
                continue;
            }

            let skip = if idx == render_start {
                first_skip_rows
            } else {
                0
            };

            let visible_h = (entry.height - skip).min(remaining);

            // Paint the background across the full width
            if let Some(bg) = entry.bg {
                for row in y..y + visible_h {
                    frame.render_widget(
                        Paragraph::new("").style(Style::default().bg(bg)),
                        Rect::new(area.x, row, area.width, 1),
                    );
                }
            }

            // Render the left border stripe (Crush/OpenCode style)
            if entry.show_border && !entry.border_char.is_empty() {
                for row in y..y + visible_h {
                    let border_span = Span::styled(
                        format!("{} ", entry.border_char),
                        entry.border_style,
                    );
                    frame.render_widget(
                        Paragraph::new(Line::from(border_span)),
                        Rect::new(area.x, row, BORDER_WIDTH, 1),
                    );
                }
            }

            // Render the wrapped text (offset by border width)
            let text_x = if entry.show_border { area.x + BORDER_WIDTH } else { area.x + BORDER_WIDTH };
            let text_w = area.width.saturating_sub(BORDER_WIDTH);
            
            let mut para = Paragraph::new(entry.text.clone())
                .wrap(Wrap { trim: false })
                .scroll((skip, 0));

            if let Some(bg) = entry.bg {
                para = para.style(Style::default().bg(bg));
            }

            frame.render_widget(para, Rect::new(text_x, y, text_w, visible_h));

            y += visible_h;
            remaining -= visible_h;
        }

        Ok(())
    }
}
