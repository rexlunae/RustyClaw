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

pub struct MessagesPane {
    focused: bool,
    /// Vertical scroll offset in visual (wrapped) lines from the bottom.
    /// `usize::MAX` = pinned to the newest content (auto-scroll).
    scroll_offset: usize,
}

impl MessagesPane {
    pub fn new(focused: bool, _focused_border_style: Style) -> Self {
        Self {
            focused,
            scroll_offset: usize::MAX,
        }
    }

    /// Map a [`MessageRole`] to its foreground colour.
    fn role_color(role: &MessageRole) -> Color {
        match role {
            MessageRole::User => tp::ACCENT_BRIGHT,
            MessageRole::Assistant => tp::TEXT,
            MessageRole::Info => tp::INFO,
            MessageRole::Success => tp::SUCCESS,
            MessageRole::Warning => tp::WARN,
            MessageRole::Error => tp::ERROR,
            MessageRole::System => tp::MUTED,
        }
    }

    /// Map a [`MessageRole`] to an optional subtle background colour.
    fn role_bg(role: &MessageRole) -> Option<Color> {
        match role {
            MessageRole::User => Some(tp::BG_USER),
            MessageRole::Assistant => Some(tp::BG_ASSISTANT),
            _ => None,
        }
    }

    /// Whether this role should show a leading icon.
    ///
    /// User and Assistant rely on background colour instead of an icon.
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

    /// Parse inline markdown into styled [`Span`]s.
    ///
    /// Supports: **bold**, *italic*, `code`, and ### headings (prefix only).
    fn parse_inline_markdown(text: &str, base_color: Color) -> Vec<Span<'static>> {
        let mut spans = Vec::new();

        // Handle heading prefixes
        let text = if text.starts_with("### ") {
            spans.push(Span::styled(
                "▎ ",
                Style::default().fg(tp::ACCENT_DIM),
            ));
            &text[4..]
        } else if text.starts_with("## ") {
            spans.push(Span::styled(
                "▎ ",
                Style::default()
                    .fg(tp::ACCENT_BRIGHT)
                    .add_modifier(Modifier::BOLD),
            ));
            &text[3..]
        } else if text.starts_with("# ") {
            spans.push(Span::styled(
                "▎ ",
                Style::default()
                    .fg(tp::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ));
            &text[2..]
        } else {
            text
        };

        let chars: Vec<char> = text.chars().collect();
        let len = chars.len();
        let mut i = 0;
        let mut buf = String::new();

        let base = Style::default().fg(base_color);
        let bold = base.add_modifier(Modifier::BOLD);
        let italic = base.add_modifier(Modifier::ITALIC);
        let code = Style::default()
            .fg(tp::ACCENT_BRIGHT)
            .bg(tp::SURFACE_BRIGHT);

        while i < len {
            // Backtick code
            if chars[i] == '`' {
                if !buf.is_empty() {
                    spans.push(Span::styled(buf.clone(), base));
                    buf.clear();
                }
                i += 1;
                let start = i;
                while i < len && chars[i] != '`' {
                    i += 1;
                }
                let code_text: String = chars[start..i].iter().collect();
                spans.push(Span::styled(format!(" {} ", code_text), code));
                if i < len {
                    i += 1; // skip closing `
                }
                continue;
            }

            // **bold**
            if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
                if !buf.is_empty() {
                    spans.push(Span::styled(buf.clone(), base));
                    buf.clear();
                }
                i += 2;
                let start = i;
                while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '*') {
                    i += 1;
                }
                let bold_text: String = chars[start..i].iter().collect();
                spans.push(Span::styled(bold_text, bold));
                if i + 1 < len {
                    i += 2; // skip closing **
                }
                continue;
            }

            // *italic*
            if chars[i] == '*' {
                if !buf.is_empty() {
                    spans.push(Span::styled(buf.clone(), base));
                    buf.clear();
                }
                i += 1;
                let start = i;
                while i < len && chars[i] != '*' {
                    i += 1;
                }
                let italic_text: String = chars[start..i].iter().collect();
                spans.push(Span::styled(italic_text, italic));
                if i < len {
                    i += 1; // skip closing *
                }
                continue;
            }

            buf.push(chars[i]);
            i += 1;
        }

        if !buf.is_empty() {
            spans.push(Span::styled(buf, base));
        }

        spans
    }

    // ── Layout helpers ──────────────────────────────────────────────────

    /// Build a styled [`Line`] for a single message.  Word-wrapping is
    /// handled by the `Paragraph` widget at render time.
    fn build_line(msg: &DisplayMessage) -> Line<'static> {
        let color = Self::role_color(&msg.role);
        let mut spans: Vec<Span<'static>> = Vec::new();

        // Left padding
        spans.push(Span::raw(" "));

        // Icon for non-chat roles only (user/assistant use bg colours)
        if Self::should_show_icon(&msg.role) {
            let icon = msg.role.icon();
            spans.push(Span::styled(
                format!("{icon} "),
                Style::default().fg(color),
            ));
        }

        // Content — parse markdown for assistant, plain for everything else
        if matches!(msg.role, MessageRole::Assistant) {
            spans.extend(Self::parse_inline_markdown(&msg.content, color));
        } else {
            spans.push(Span::styled(
                msg.content.clone(),
                Style::default().fg(color),
            ));
        }

        Line::from(spans)
    }

    /// Count how many visual (wrapped) rows a `Line` occupies at `width`.
    fn visual_line_count(line: &Line<'_>, width: u16) -> u16 {
        if width == 0 {
            return 1;
        }
        let w = width as usize;
        let text_width: usize = line.width();
        if text_width == 0 {
            return 1;
        }
        ((text_width + w - 1) / w) as u16
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
            let line = Self::build_line(msg);
            let h = Self::visual_line_count(&line, width) as usize;
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
                // We don't know the real render width here, so we use a
                // conservative estimate; the actual clamping happens in draw.
                let spacing = state.config.message_spacing;
                let msg_count = state.messages.len();
                let total: usize = state
                    .messages
                    .iter()
                    .map(|m| {
                        let l = Self::build_line(m);
                        Self::visual_line_count(&l, 200) as usize
                    })
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

        // ── Build entries with pre-computed visual heights ───────────

        struct Entry<'a> {
            line: Line<'a>,
            bg: Option<Color>,
            height: u16,
        }

        let spacing = state.config.message_spacing;

        let mut entries: Vec<Entry<'_>> = Vec::new();
        for (i, msg) in state.messages.iter().enumerate() {
            // Insert blank spacing line(s) between messages
            if i > 0 && spacing > 0 {
                entries.push(Entry {
                    line: Line::from(""),
                    bg: None,
                    height: spacing,
                });
            }
            let line = Self::build_line(msg);
            let h = Self::visual_line_count(&line, width);
            entries.push(Entry {
                line,
                bg: Self::role_bg(&msg.role),
                height: h,
            });
        }

        // Append loading line if active
        if let Some(ref loading) = state.loading_line {
            let line = Line::from(Span::styled(
                format!(" {}", loading),
                Style::default().fg(tp::ACCENT_BRIGHT),
            ));
            let h = Self::visual_line_count(&line, width);
            entries.push(Entry {
                line,
                bg: None,
                height: h,
            });
        }

        let total_visual: usize = entries.iter().map(|e| e.height as usize).sum();
        let viewport = area.height as usize;

        // ── Resolve scroll position ─────────────────────────────────
        // `scroll_offset` is "lines from the bottom":
        //   usize::MAX or 0 → pinned to the newest content
        //   >0 → scrolled up by that many visual lines

        let max_scroll = total_visual.saturating_sub(viewport);

        let from_bottom = if self.scroll_offset == usize::MAX {
            0
        } else {
            self.scroll_offset.min(max_scroll)
        };
        // Persist the clamped value so Up/Down work correctly.
        if self.scroll_offset != usize::MAX {
            self.scroll_offset = from_bottom;
        }

        // `scroll_top` = number of visual lines to skip from the top.
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

            // Render the wrapped text
            let mut para = Paragraph::new(entry.line.clone())
                .wrap(Wrap { trim: false })
                .scroll((skip, 0));

            if let Some(bg) = entry.bg {
                para = para.style(Style::default().bg(bg));
            }

            frame.render_widget(para, Rect::new(area.x, y, area.width, visible_h));

            y += visible_h;
            remaining -= visible_h;
        }

        Ok(())
    }
}

