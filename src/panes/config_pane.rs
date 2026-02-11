use anyhow::Result;
use ratatui::{
    layout::{Constraint, Rect},
    prelude::*,
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
};

use crate::action::Action;
use crate::panes::{Pane, PaneState};
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
            Style::default()
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
            Some(m) => format!(
                "Provider: {}  Model: {}",
                m.provider,
                m.model.as_deref().unwrap_or("(none)")
            ),
            None => "(not configured — run `rustyclaw onboard`)".to_string(),
        };

        let text = [
            format!(
                "Settings Directory: {}",
                state.config.settings_dir.display()
            ),
            format!(
                "Workspace: {}",
                state.config.workspace_dir().display()
            ),
            format!("Model: {}", model_info),
            format!("SOUL Path: {}", state.soul_manager.get_path().display()),
            format!("Use Secrets: {}", state.config.use_secrets),
            String::new(),
            "SOUL Content Preview:".to_string(),
            soul_content
                .lines()
                .take(10)
                .collect::<Vec<_>>()
                .join("\n"),
        ];

        let config_text = text.join("\n");
        let config_para = Paragraph::new(config_text)
            .block(
                Block::default()
                    .title("Configuration")
                    .borders(Borders::ALL)
                    .border_style(self.border_style())
                    .border_type(self.border_type()),
            )
            .wrap(Wrap { trim: true });

        frame.render_widget(config_para, area);
        Ok(())
    }
}
