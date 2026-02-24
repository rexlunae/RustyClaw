use anyhow::Result;
use ratatui::{
    layout::{Constraint, Rect},
    prelude::*,
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
};

use crate::action::Action;
use crate::panes::{Pane, PaneState};
use crate::tui_palette as tp;
use crate::tui::Frame;

pub struct ConfigPane {
    focused: bool,
    focused_border_style: Style,
}

impl ConfigPane {
    pub fn new(focused: bool, focused_border_style: Style) -> Self {
        Self {
            focused,
            focused_border_style,
        }
    }

    fn border_style(&self) -> Style {
        if self.focused {
            self.focused_border_style
        } else {
            tp::unfocused_border()
        }
    }

    fn border_type(&self) -> BorderType {
        if self.focused {
            BorderType::Thick
        } else {
            BorderType::Plain
        }
    }
}

impl Pane for ConfigPane {
    fn height_constraint(&self) -> Constraint {
        match self.focused {
            true => Constraint::Fill(3),
            false => Constraint::Fill(1),
        }
    }

    fn update(&mut self, action: Action, _state: &mut PaneState<'_>) -> Result<Option<Action>> {
        match action {
            Action::Focus => {
                self.focused = true;
                return Ok(Some(Action::TimedStatusLine(
                    "[config pane focused]".into(),
                    3,
                )));
            }
            Action::UnFocus => {
                self.focused = false;
            }
            _ => {}
        }
        Ok(None)
    }

    fn draw(&mut self, frame: &mut Frame<'_>, area: Rect, state: &PaneState<'_>) -> Result<()> {
        let soul_content = state
            .soul_manager
            .get_content()
            .unwrap_or("No SOUL content loaded");

        let model_info = match &state.config.model {
            Some(m) => vec![
                Span::styled(&m.provider, Style::default().fg(tp::ACCENT_BRIGHT)),
                Span::styled(" / ", Style::default().fg(tp::MUTED)),
                Span::styled(
                    m.model.as_deref().unwrap_or("(none)"),
                    Style::default().fg(tp::INFO),
                ),
            ],
            None => vec![Span::styled(
                "(not configured — run rustyclaw onboard)",
                Style::default().fg(tp::WARN),
            )],
        };

        let settings_str = state.config.settings_dir.display().to_string();
        let workspace_str = state.config.workspace_dir().display().to_string();
        let soul_path_str = state.soul_manager.get_path().display().to_string();
        let secrets_str = if state.config.use_secrets { "enabled" } else { "disabled" };

        fn kv<'a>(key: &'a str, val: &'a str) -> Line<'a> {
            Line::from(vec![
                Span::styled(key, Style::default().fg(tp::TEXT_DIM)),
                Span::styled(val, Style::default().fg(tp::INFO)),
            ])
        }

        let mut lines: Vec<Line> = vec![
            kv("Settings : ", &settings_str),
            kv("Workspace: ", &workspace_str),
        ];

        // Model line with rich spans
        let mut model_line_spans = vec![Span::styled("Model    : ", Style::default().fg(tp::TEXT_DIM))];
        model_line_spans.extend(model_info);
        lines.push(Line::from(model_line_spans));

        lines.push(kv("SOUL Path: ", &soul_path_str));
        lines.push(kv("Secrets  : ", secrets_str));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "SOUL Content Preview:",
            Style::default().fg(tp::ACCENT).add_modifier(Modifier::BOLD),
        )));

        for line in soul_content.lines().take(10) {
            let style = if line.starts_with('#') {
                Style::default().fg(tp::ACCENT_BRIGHT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(tp::TEXT_DIM)
            };
            lines.push(Line::from(Span::styled(line, style)));
        }

        let title_style = if self.focused {
            tp::title_focused()
        } else {
            tp::title_unfocused()
        };

        let config_para = Paragraph::new(lines)
            .block(
                Block::default()
                    .title(Span::styled(" Configuration ", title_style))
                    .borders(Borders::ALL)
                    .border_style(self.border_style())
                    .border_type(self.border_type()),
            )
            .wrap(Wrap { trim: true });

        frame.render_widget(config_para, area);
        Ok(())
    }
}
