use anyhow::Result;
use ratatui::{
    layout::{Constraint, Layout, Direction, Rect},
    prelude::*,
};

use crate::panes::{Pane, PaneState};
use crate::tui_palette as tp;
use crate::tui::Frame;

#[derive(Default)]
pub struct HeaderPane {}

impl HeaderPane {
    pub fn new() -> Self {
        Self {}
    }
}

impl Pane for HeaderPane {
    fn height_constraint(&self) -> Constraint {
        Constraint::Length(3)
    }

    fn draw(&mut self, frame: &mut Frame<'_>, area: Rect, state: &PaneState<'_>) -> Result<()> {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Line 1: soul name + secrets
                Constraint::Length(1), // Line 2: model + gateway url
                Constraint::Length(1), // Line 3: separator
            ])
            .split(area);

        // ── Line 1: soul name (left) ─ secrets status (right) ───────────
        let soul_name = state
            .soul_manager
            .get_content()
            .and_then(|c| {
                c.lines()
                    .find(|l| l.starts_with("# "))
                    .map(|l| l.trim_start_matches("# ").to_string())
            })
            .unwrap_or_else(|| "RustyClaw".to_string());

        let left1 = Line::from(vec![
            Span::styled(" 🦀 ", Style::default().fg(tp::ACCENT)),
            Span::styled(
                &soul_name,
                Style::default()
                    .fg(tp::ACCENT_BRIGHT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {} ", symbols::DOT),
                Style::default().fg(tp::MUTED),
            ),
            Span::styled(
                format!("v{}", env!("CARGO_PKG_VERSION")),
                Style::default().fg(tp::INFO),
            ),
        ]);

        let (secrets_label, secrets_style) = if state.config.use_secrets {
            ("● secrets", Style::default().fg(tp::SUCCESS))
        } else {
            ("○ secrets", Style::default().fg(tp::MUTED))
        };

        let right1 = Line::from(vec![Span::styled(secrets_label, secrets_style), Span::raw(" ")]);

        // Render left and right on line 1 by splitting horizontally
        let cols1 = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Fill(1), Constraint::Min(12)])
            .split(rows[0]);
        frame.render_widget(left1, cols1[0]);
        frame.render_widget(right1.right_aligned(), cols1[1]);

        // ── Line 2: model info (left) ─ gateway url (right) ─────────────
        let model_spans: Vec<Span> = match &state.config.model {
            Some(m) => vec![
                Span::styled(" Model: ", Style::default().fg(tp::TEXT_DIM)),
                Span::styled(&m.provider, Style::default().fg(tp::ACCENT_BRIGHT)),
                Span::styled(" / ", Style::default().fg(tp::MUTED)),
                Span::styled(
                    m.model.as_deref().unwrap_or("(default)"),
                    Style::default().fg(tp::INFO),
                ),
            ],
            None => vec![Span::styled(
                " Model: (not configured — run rustyclaw onboard)",
                Style::default().fg(tp::WARN),
            )],
        };
        let left2 = Line::from(model_spans);

        let gw_url = state
            .config
            .gateway_url
            .as_deref()
            .unwrap_or("ws://127.0.0.1:9001");
        let right2 = Line::from(vec![
            Span::styled("⇄ ", Style::default().fg(tp::MUTED)),
            Span::styled(gw_url, Style::default().fg(tp::TEXT_DIM)),
            Span::raw(" "),
        ]);

        let cols2 = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Fill(1), Constraint::Min(24)])
            .split(rows[1]);
        frame.render_widget(left2, cols2[0]);
        frame.render_widget(right2.right_aligned(), cols2[1]);

        // ── Line 3: thin accent separator ───────────────────────────────
        let sep = "─".repeat(area.width as usize);
        frame.render_widget(
            Line::from(Span::styled(sep, Style::default().fg(tp::ACCENT_DIM))),
            rows[2],
        );

        Ok(())
    }
}
