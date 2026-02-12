use std::sync::OnceLock;

use anyhow::Result;
use ratatui::{
    layout::{Constraint, Rect},
    prelude::*,
    widgets::{Paragraph, Wrap},
};
use syntect::easy::HighlightLines;
use syntect::highlighting::{self, ThemeSet};
use syntect::parsing::SyntaxSet;

use crate::action::Action;
use crate::panes::{DisplayMessage, MessageRole, Pane, PaneState};
use crate::theme::tui_palette as tp;
use crate::tui::Frame;

// ── Lazy-loaded syntect state ───────────────────────────────────────────

fn syntax_set() -> &'static SyntaxSet {
    static SS: OnceLock<SyntaxSet> = OnceLock::new();
    SS.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn highlight_theme() -> &'static highlighting::Theme {
    static TH: OnceLock<highlighting::Theme> = OnceLock::new();
    TH.get_or_init(|| {
        let ts = ThemeSet::load_defaults();
        ts.themes["base16-ocean.dark"].clone()
    })
}

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

    // ── Syntax highlighting ─────────────────────────────────────────────

    /// Convert a syntect `highlighting::Style` to a ratatui `Style`.
    fn syntect_to_ratatui(ss: highlighting::Style) -> Style {
        let fg = Color::Rgb(ss.foreground.r, ss.foreground.g, ss.foreground.b);
        Style::default().fg(fg).bg(tp::BG_CODE)
    }

    /// Syntax-highlight a block of code lines using syntect.
    fn highlight_code_block(lines: &[&str], lang: &str) -> Vec<Line<'static>> {
        let ss = syntax_set();
        let theme = highlight_theme();

        let syntax = ss
            .find_syntax_by_token(lang)
            .unwrap_or_else(|| ss.find_syntax_plain_text());

        let mut h = HighlightLines::new(syntax, theme);
        let mut result = Vec::new();

        for source_line in lines {
            // syntect expects the newline; add it back for parsing
            let input = format!("{source_line}\n");
            let ranges = h
                .highlight_line(&input, ss)
                .unwrap_or_default();

            let mut spans: Vec<Span<'static>> = Vec::new();
            // Left gutter
            spans.push(Span::styled("  ", Style::default().bg(tp::BG_CODE)));
            for (style, text) in ranges {
                // Strip the trailing newline we added
                let t = text.trim_end_matches('\n').to_string();
                if !t.is_empty() {
                    spans.push(Span::styled(t, Self::syntect_to_ratatui(style)));
                }
            }
            result.push(Line::from(spans));
        }

        result
    }

    // ── Layout helpers ──────────────────────────────────────────────────

    /// Build styled [`Line`]s for a single message.
    ///
    /// Multi-line content (e.g. assistant responses) is split on `\n`.
    /// Fenced code blocks (` ```lang … ``` `) get syntax highlighting.
    fn build_lines(msg: &DisplayMessage) -> Vec<Line<'static>> {
        let color = Self::role_color(&msg.role);
        let is_assistant = matches!(msg.role, MessageRole::Assistant);

        if !is_assistant {
            // Non-assistant messages stay single-line.
            let mut spans: Vec<Span<'static>> = Vec::new();
            spans.push(Span::raw(" "));
            if Self::should_show_icon(&msg.role) {
                let icon = msg.role.icon();
                spans.push(Span::styled(
                    format!("{icon} "),
                    Style::default().fg(color),
                ));
            }
            spans.push(Span::styled(
                msg.content.clone(),
                Style::default().fg(color),
            ));
            return vec![Line::from(spans)];
        }

        // ── Assistant: handle multi-line with code fences ────────────

        let raw_lines: Vec<&str> = msg.content.split('\n').collect();
        let mut result: Vec<Line<'static>> = Vec::new();
        let mut i = 0;

        while i < raw_lines.len() {
            let line = raw_lines[i];

            // Detect opening code fence: ```lang
            if line.trim_start().starts_with("```") {
                let trimmed = line.trim_start();
                let lang = trimmed[3..].trim().to_string();

                // Fence header line (dimmed)
                let fence_label = if lang.is_empty() {
                    " ─── code ───".to_string()
                } else {
                    format!(" ─── {} ───", lang)
                };
                result.push(Line::from(Span::styled(
                    fence_label,
                    Style::default().fg(tp::MUTED).bg(tp::BG_CODE),
                )));

                // Collect code body
                i += 1;
                let mut code_lines: Vec<&str> = Vec::new();
                while i < raw_lines.len() {
                    if raw_lines[i].trim_start().starts_with("```") {
                        break;
                    }
                    code_lines.push(raw_lines[i]);
                    i += 1;
                }

                // Highlight the code body
                let lang_ref = if lang.is_empty() { "txt" } else { &lang };
                result.extend(Self::highlight_code_block(&code_lines, lang_ref));

                // Closing fence line
                result.push(Line::from(Span::styled(
                    " ───────────",
                    Style::default().fg(tp::MUTED).bg(tp::BG_CODE),
                )));

                // Skip closing ``` line
                if i < raw_lines.len() {
                    i += 1;
                }
                continue;
            }

            // Normal markdown line
            let mut spans = Vec::new();
            spans.push(Span::raw(" "));
            spans.extend(Self::parse_inline_markdown(line, color));
            result.push(Line::from(spans));
            i += 1;
        }

        result
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
                    (text_width + w - 1) / w
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
            let lines = Self::build_lines(msg);
            let h = Self::visual_lines_count(&lines, width);
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
                    .map(|m| Self::visual_lines_count(&Self::build_lines(m), 200))
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
            /// All rendered lines for this entry (may be many for multi-line messages).
            text: Text<'a>,
            bg: Option<Color>,
            /// Total visual rows after wrapping.
            height: u16,
        }

        let spacing = state.config.message_spacing;

        let mut entries: Vec<Entry<'_>> = Vec::new();
        for (i, msg) in state.messages.iter().enumerate() {
            // Insert blank spacing line(s) between messages
            if i > 0 && spacing > 0 {
                entries.push(Entry {
                    text: Text::from(""),
                    bg: None,
                    height: spacing,
                });
            }
            let lines = Self::build_lines(msg);
            let h = Self::visual_lines_count(&lines, width) as u16;
            entries.push(Entry {
                text: Text::from(lines),
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
            let h = Self::visual_lines_count(&[line.clone()], width) as u16;
            entries.push(Entry {
                text: Text::from(line),
                bg: None,
                height: h,
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

            // Render the wrapped text
            let mut para = Paragraph::new(entry.text.clone())
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
