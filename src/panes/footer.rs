use std::time::Instant;

use anyhow::Result;
use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{prelude::*, widgets::Paragraph};
use tui_input::{backend::crossterm::EventHandler, Input};

use crate::action::Action;
use crate::commands::command_names;
use crate::panes::{GatewayStatus, InputMode, Pane, PaneState};
use crate::theme::tui_palette as tp;
use crate::tui::{EventResponse, Frame};

struct TimedStatusLine {
    message: String,
    expires: Instant,
}

/// Always-visible input bar at the bottom of the screen.
///
/// - Plain text is submitted as a prompt.
/// - Text beginning with `/` is treated as a command.
/// - While typing a `/` command a tab-completion dropdown appears above.
pub struct FooterPane {
    input: Input,
    timed_status: Option<TimedStatusLine>,
    command_history: Vec<String>,
    history_index: Option<usize>,
    /// Filtered completions for the current `/` prefix
    completions: Vec<String>,
    /// Currently highlighted completion index (None = nothing selected)
    completion_index: Option<usize>,
    /// Whether the completion popup is visible
    show_completions: bool,
}

impl Default for FooterPane {
    fn default() -> Self {
        Self {
            input: Input::default(),
            timed_status: None,
            command_history: Vec::new(),
            history_index: None,
            completions: Vec::new(),
            completion_index: None,
            show_completions: false,
        }
    }
}

impl FooterPane {
    pub fn new() -> Self {
        Self::default()
    }

    /// Recalculate the completions list from the current input value.
    fn update_completions(&mut self) {
        let val = self.input.value();
        if val.starts_with('/') {
            let partial = &val[1..]; // text after '/'
            let names = command_names();
            self.completions = names
                .iter()
                .filter(|c| c.starts_with(partial))
                .map(|c| c.to_string())
                .collect();
            self.show_completions = !self.completions.is_empty();
        } else {
            self.completions.clear();
            self.show_completions = false;
        }
        // Reset selection whenever the list changes
        self.completion_index = None;
    }

    /// Apply the currently selected completion into the input.
    fn apply_completion(&mut self) {
        if let Some(idx) = self.completion_index {
            if let Some(cmd) = self.completions.get(idx) {
                self.input = Input::new(format!("/{}", cmd));
                self.show_completions = false;
                self.completions.clear();
                self.completion_index = None;
            }
        }
    }

    /// Height of the completion popup (capped at 10 rows).
    pub fn completion_popup_height(&self) -> u16 {
        if self.show_completions {
            (self.completions.len() as u16).min(10)
        } else {
            0
        }
    }
}

impl Pane for FooterPane {
    fn height_constraint(&self) -> Constraint {
        // 1 row for status, 1 row for the input line
        Constraint::Length(2)
    }

    fn handle_key_events(
        &mut self,
        key: KeyEvent,
        state: &mut PaneState<'_>,
    ) -> Result<Option<EventResponse<Action>>> {
        // Ctrl-C always quits
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Ok(Some(EventResponse::Stop(Action::Quit)));
        }

        match state.input_mode {
            InputMode::Input => {
                match key.code {
                    KeyCode::Enter => {
                        // If a completion is highlighted, apply it first
                        if self.show_completions && self.completion_index.is_some() {
                            self.apply_completion();
                        }
                        let value = self.input.value().to_string();
                        self.input.reset();
                        self.show_completions = false;
                        self.completions.clear();
                        self.completion_index = None;
                        state.input_mode = InputMode::Normal;
                        if !value.is_empty() {
                            self.command_history.push(value.clone());
                        }
                        self.history_index = None;
                        if !value.is_empty() {
                            return Ok(Some(EventResponse::Stop(Action::InputSubmit(value))));
                        }
                        return Ok(Some(EventResponse::Stop(Action::Noop)));
                    }
                    KeyCode::Esc => {
                        self.input.reset();
                        self.show_completions = false;
                        self.completions.clear();
                        self.completion_index = None;
                        self.history_index = None;
                        state.input_mode = InputMode::Normal;
                        return Ok(Some(EventResponse::Stop(Action::Noop)));
                    }
                    KeyCode::Tab => {
                        if self.show_completions && !self.completions.is_empty() {
                            // Cycle forward through completions
                            self.completion_index = Some(match self.completion_index {
                                Some(i) => (i + 1) % self.completions.len(),
                                None => 0,
                            });
                            self.apply_completion();
                            // re-open list for further tabbing
                            self.update_completions();
                        } else if self.input.value().starts_with('/') {
                            // Open completions
                            self.update_completions();
                        }
                        return Ok(Some(EventResponse::Stop(Action::Noop)));
                    }
                    KeyCode::BackTab => {
                        if self.show_completions && !self.completions.is_empty() {
                            self.completion_index = Some(match self.completion_index {
                                Some(0) | None => self.completions.len() - 1,
                                Some(i) => i - 1,
                            });
                            self.apply_completion();
                            self.update_completions();
                        }
                        return Ok(Some(EventResponse::Stop(Action::Noop)));
                    }
                    KeyCode::Up => {
                        if self.show_completions && !self.completions.is_empty() {
                            self.completion_index = Some(match self.completion_index {
                                Some(0) | None => self.completions.len() - 1,
                                Some(i) => i - 1,
                            });
                            return Ok(Some(EventResponse::Stop(Action::Noop)));
                        }
                        // History navigation
                        if !self.command_history.is_empty() {
                            let idx = match self.history_index {
                                Some(i) => i.saturating_sub(1),
                                None => self.command_history.len() - 1,
                            };
                            self.history_index = Some(idx);
                            let val = self.command_history[idx].clone();
                            self.input = Input::new(val);
                            self.update_completions();
                        }
                        return Ok(Some(EventResponse::Stop(Action::Noop)));
                    }
                    KeyCode::Down => {
                        if self.show_completions && !self.completions.is_empty() {
                            self.completion_index = Some(match self.completion_index {
                                None => 0,
                                Some(i) => (i + 1) % self.completions.len(),
                            });
                            return Ok(Some(EventResponse::Stop(Action::Noop)));
                        }
                        // History navigation
                        if let Some(idx) = self.history_index {
                            if idx + 1 < self.command_history.len() {
                                let idx = idx + 1;
                                self.history_index = Some(idx);
                                let val = self.command_history[idx].clone();
                                self.input = Input::new(val);
                            } else {
                                self.history_index = None;
                                self.input.reset();
                            }
                            self.update_completions();
                        }
                        return Ok(Some(EventResponse::Stop(Action::Noop)));
                    }
                    _ => {
                        self.input.handle_event(&CrosstermEvent::Key(key));
                        self.update_completions();
                        return Ok(Some(EventResponse::Stop(Action::Noop)));
                    }
                }
            }
            InputMode::Normal => {
                // Any printable character starts typing
                if let KeyCode::Char(c) = key.code {
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT)
                    {
                        state.input_mode = InputMode::Input;
                        self.input.handle_event(&CrosstermEvent::Key(key));
                        if c == '/' {
                            self.update_completions();
                        }
                        return Ok(Some(EventResponse::Stop(Action::Noop)));
                    }
                }
                // Don't consume other keys in Normal mode — let pages handle them
                return Ok(None);
            }
        }
    }

    fn update(&mut self, action: Action, _state: &mut PaneState<'_>) -> Result<Option<Action>> {
        match action {
            Action::StatusLine(msg) => {
                self.timed_status = Some(TimedStatusLine {
                    message: msg,
                    expires: Instant::now() + std::time::Duration::from_secs(60),
                });
            }
            Action::TimedStatusLine(msg, secs) => {
                self.timed_status = Some(TimedStatusLine {
                    message: msg,
                    expires: Instant::now() + std::time::Duration::from_secs(secs),
                });
            }
            Action::Tick => {
                if let Some(ts) = &self.timed_status {
                    if Instant::now() >= ts.expires {
                        self.timed_status = None;
                    }
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn draw(&mut self, frame: &mut Frame<'_>, area: Rect, state: &PaneState<'_>) -> Result<()> {
        // We get a 2-row area: top row = status, bottom row = input
        if area.height < 2 {
            // Degenerate — just draw input
            self.draw_input(frame, area, state);
            return Ok(());
        }

        let status_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        };
        let input_area = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: 1,
        };

        // Status line
        let msg = self
            .timed_status
            .as_ref()
            .map(|ts| ts.message.as_str())
            .unwrap_or("[ESC → navigate panes] [TAB → complete command] [/help]");

        frame.render_widget(
            Paragraph::new(msg).style(tp::hint()),
            status_area,
        );

        self.draw_input(frame, input_area, state);

        // Completion popup — rendered on top of the body area, just above the footer
        if self.show_completions && !self.completions.is_empty() {
            let popup_h = self.completion_popup_height();
            let popup_y = area.y.saturating_sub(popup_h);
            let popup_w = 44.min(area.width);
            let popup_area = Rect {
                x: area.x + 2, // align with input text start
                y: popup_y,
                width: popup_w,
                height: popup_h,
            };

            // Clear background
            frame.render_widget(
                ratatui::widgets::Clear,
                popup_area,
            );

            let items: Vec<Line> = self
                .completions
                .iter()
                .enumerate()
                .map(|(i, cmd)| {
                    let style = if Some(i) == self.completion_index {
                        tp::popup_selected()
                    } else {
                        tp::popup_item()
                    };
                    Line::from(Span::styled(format!(" /{} ", cmd), style))
                })
                .collect();

            frame.render_widget(
                Paragraph::new(items)
                    .block(
                        ratatui::widgets::Block::default()
                            .borders(ratatui::widgets::Borders::NONE),
                    )
                    .style(tp::popup_bg()),
                popup_area,
            );
        }

        Ok(())
    }
}

impl FooterPane {
    fn draw_input(&self, frame: &mut Frame<'_>, area: Rect, state: &PaneState<'_>) {
        let prefix = if state.input_mode == InputMode::Input {
            Span::styled("❯ ", tp::prompt_active())
        } else {
            Span::styled("❯ ", tp::prompt_inactive())
        };

        // Gateway status indicator (right-aligned)
        let (status_icon, status_label, status_style) = match state.gateway_status {
            GatewayStatus::Connected => (
                "● ",
                "connected ",
                Style::default().fg(tp::SUCCESS),
            ),
            GatewayStatus::ModelReady => (
                "● ",
                "model ready ",
                Style::default().fg(tp::SUCCESS).add_modifier(Modifier::BOLD),
            ),
            GatewayStatus::Connecting => (
                "◌ ",
                "connecting… ",
                Style::default().fg(tp::WARN),
            ),
            GatewayStatus::Disconnected => (
                "○ ",
                "disconnected ",
                Style::default().fg(tp::ERROR),
            ),
            GatewayStatus::ModelError => (
                "✖ ",
                "model error ",
                Style::default().fg(tp::ERROR).add_modifier(Modifier::BOLD),
            ),
            GatewayStatus::Error => (
                "✖ ",
                "error ",
                Style::default().fg(tp::ERROR).add_modifier(Modifier::BOLD),
            ),
            GatewayStatus::Unconfigured => (
                "○ ",
                "no gateway ",
                Style::default().fg(tp::MUTED),
            ),
        };

        let status_width = (status_icon.len() + status_label.len()) as u16;
        let prefix_width: u16 = 2; // "❯ "
        let input_width = area.width.saturating_sub(prefix_width + status_width);
        let scroll = self.input.visual_scroll(input_width as usize);
        let input_text = self.input.value();

        // Left side: prompt prefix + input text
        let input_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width.saturating_sub(status_width),
            height: 1,
        };
        let line = Line::from(vec![
            prefix,
            Span::raw(&input_text[scroll..]),
        ]);
        frame.render_widget(Paragraph::new(line), input_area);

        // Right side: gateway status
        let status_area = Rect {
            x: area.x + area.width.saturating_sub(status_width),
            y: area.y,
            width: status_width,
            height: 1,
        };
        let status_line = Line::from(vec![
            Span::styled(status_icon, status_style),
            Span::styled(status_label, status_style),
        ]);
        frame.render_widget(Paragraph::new(status_line), status_area);

        // Show cursor when in input mode
        if state.input_mode == InputMode::Input {
            frame.set_cursor_position((
                area.x + prefix_width + self.input.visual_cursor().saturating_sub(scroll) as u16,
                area.y,
            ));
        }
    }
}
