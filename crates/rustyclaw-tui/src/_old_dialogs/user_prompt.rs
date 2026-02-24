//! User prompt dialog for structured agent questions.
//!
//! Shown when the agent calls the `ask_user` tool — renders a TUI dialog
//! for Select, MultiSelect, Confirm, TextInput, or Form prompts.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::action::Action;
use crate::tui_palette as tp;

// Re-export shared prompt types from core
pub use rustyclaw_core::user_prompt_types::{
    FormField, PromptOption, PromptType, UserPrompt, UserPromptResponse,
};

// ── Dialog state ────────────────────────────────────────────────────────────

/// TUI state for the user prompt dialog.
pub struct UserPromptState {
    pub prompt: UserPrompt,
    pub phase: PromptPhase,
}

pub enum PromptPhase {
    Select {
        cursor: usize,
        scroll: usize,
    },
    MultiSelect {
        cursor: usize,
        scroll: usize,
        selected: Vec<bool>,
    },
    Confirm {
        yes: bool,
    },
    TextInput {
        input: String,
    },
    Form {
        cursor: usize,
        inputs: Vec<String>,
    },
}

impl UserPromptState {
    pub fn new(prompt: UserPrompt) -> Self {
        let phase = match &prompt.prompt_type {
            PromptType::Select { default, options } => PromptPhase::Select {
                cursor: default.unwrap_or(0).min(options.len().saturating_sub(1)),
                scroll: 0,
            },
            PromptType::MultiSelect { defaults, options } => {
                let mut selected = vec![false; options.len()];
                for &i in defaults {
                    if i < selected.len() {
                        selected[i] = true;
                    }
                }
                PromptPhase::MultiSelect {
                    cursor: 0,
                    scroll: 0,
                    selected,
                }
            }
            PromptType::Confirm { default } => PromptPhase::Confirm { yes: *default },
            PromptType::TextInput { default, .. } => PromptPhase::TextInput {
                input: default.clone().unwrap_or_default(),
            },
            PromptType::Form { fields } => PromptPhase::Form {
                cursor: 0,
                inputs: fields
                    .iter()
                    .map(|f| f.default.clone().unwrap_or_default())
                    .collect(),
            },
        };
        Self { prompt, phase }
    }

    fn build_response(&self, dismissed: bool) -> UserPromptResponse {
        let value = if dismissed {
            serde_json::Value::Null
        } else {
            match &self.phase {
                PromptPhase::Select { cursor, .. } => {
                    if let PromptType::Select { options, .. } = &self.prompt.prompt_type {
                        let opt = &options[*cursor];
                        serde_json::json!(opt.value.as_deref().unwrap_or(&opt.label))
                    } else {
                        serde_json::Value::Null
                    }
                }
                PromptPhase::MultiSelect { selected, .. } => {
                    if let PromptType::MultiSelect { options, .. } = &self.prompt.prompt_type {
                        let vals: Vec<String> = options
                            .iter()
                            .zip(selected.iter())
                            .filter(|(_, s)| **s)
                            .map(|(opt, _)| {
                                opt.value.clone().unwrap_or_else(|| opt.label.clone())
                            })
                            .collect();
                        serde_json::json!(vals)
                    } else {
                        serde_json::Value::Null
                    }
                }
                PromptPhase::Confirm { yes } => serde_json::json!(*yes),
                PromptPhase::TextInput { input } => serde_json::json!(input),
                PromptPhase::Form { inputs, .. } => {
                    if let PromptType::Form { fields } = &self.prompt.prompt_type {
                        let mut obj = serde_json::Map::new();
                        for (field, value) in fields.iter().zip(inputs.iter()) {
                            obj.insert(
                                field.name.clone(),
                                serde_json::json!(value),
                            );
                        }
                        serde_json::Value::Object(obj)
                    } else {
                        serde_json::Value::Null
                    }
                }
            }
        };
        UserPromptResponse {
            id: self.prompt.id.clone(),
            dismissed,
            value,
        }
    }
}

// ── Key handling ────────────────────────────────────────────────────────────

pub fn handle_user_prompt_key(
    state: &mut UserPromptState,
    key: KeyEvent,
) -> Option<Action> {
    match key.code {
        KeyCode::Esc => {
            let resp = state.build_response(true);
            Some(Action::UserPromptResponse(resp))
        }
        _ => match &mut state.phase {
            PromptPhase::Select { .. } => handle_select_key(state, key.code),
            PromptPhase::MultiSelect { .. } => handle_multiselect_key(state, key.code),
            PromptPhase::Confirm { .. } => handle_confirm_key(state, key.code),
            PromptPhase::TextInput { .. } => handle_text_input_key(state, key.code),
            PromptPhase::Form { .. } => handle_form_key(state, key.code),
        },
    }
}

fn handle_select_key(state: &mut UserPromptState, code: KeyCode) -> Option<Action> {
    let option_count = match &state.prompt.prompt_type {
        PromptType::Select { options, .. } => options.len(),
        _ => return None,
    };
    if let PromptPhase::Select { cursor, .. } = &mut state.phase {
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                *cursor = cursor.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if *cursor + 1 < option_count {
                    *cursor += 1;
                }
            }
            KeyCode::Home => *cursor = 0,
            KeyCode::End => *cursor = option_count.saturating_sub(1),
            KeyCode::Enter => {
                let resp = state.build_response(false);
                return Some(Action::UserPromptResponse(resp));
            }
            _ => {}
        }
    }
    None
}

fn handle_multiselect_key(state: &mut UserPromptState, code: KeyCode) -> Option<Action> {
    let option_count = match &state.prompt.prompt_type {
        PromptType::MultiSelect { options, .. } => options.len(),
        _ => return None,
    };
    if let PromptPhase::MultiSelect {
        cursor, selected, ..
    } = &mut state.phase
    {
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                *cursor = cursor.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if *cursor + 1 < option_count {
                    *cursor += 1;
                }
            }
            KeyCode::Char(' ') => {
                if *cursor < selected.len() {
                    selected[*cursor] = !selected[*cursor];
                }
            }
            KeyCode::Char('a') => {
                let all_selected = selected.iter().all(|&s| s);
                for s in selected.iter_mut() {
                    *s = !all_selected;
                }
            }
            KeyCode::Enter => {
                let resp = state.build_response(false);
                return Some(Action::UserPromptResponse(resp));
            }
            _ => {}
        }
    }
    None
}

fn handle_confirm_key(state: &mut UserPromptState, code: KeyCode) -> Option<Action> {
    if let PromptPhase::Confirm { yes } = &mut state.phase {
        match code {
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::Char('h') | KeyCode::Char('l') => {
                *yes = !*yes;
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                *yes = true;
                let resp = state.build_response(false);
                return Some(Action::UserPromptResponse(resp));
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                *yes = false;
                let resp = state.build_response(false);
                return Some(Action::UserPromptResponse(resp));
            }
            KeyCode::Enter => {
                let resp = state.build_response(false);
                return Some(Action::UserPromptResponse(resp));
            }
            _ => {}
        }
    }
    None
}

fn handle_text_input_key(state: &mut UserPromptState, code: KeyCode) -> Option<Action> {
    if let PromptPhase::TextInput { input } = &mut state.phase {
        match code {
            KeyCode::Char(c) => input.push(c),
            KeyCode::Backspace => { input.pop(); }
            KeyCode::Enter => {
                let resp = state.build_response(false);
                return Some(Action::UserPromptResponse(resp));
            }
            _ => {}
        }
    }
    None
}

fn handle_form_key(state: &mut UserPromptState, code: KeyCode) -> Option<Action> {
    let field_count = match &state.prompt.prompt_type {
        PromptType::Form { fields } => fields.len(),
        _ => return None,
    };
    if let PromptPhase::Form { cursor, inputs } = &mut state.phase {
        match code {
            KeyCode::Up => *cursor = cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => {
                if *cursor + 1 < field_count {
                    *cursor += 1;
                }
            }
            KeyCode::Char(c) => {
                if *cursor < inputs.len() {
                    inputs[*cursor].push(c);
                }
            }
            KeyCode::Backspace => {
                if *cursor < inputs.len() {
                    inputs[*cursor].pop();
                }
            }
            KeyCode::Enter => {
                // Tab to next field, or submit on last field
                if *cursor + 1 < field_count {
                    *cursor += 1;
                } else {
                    let resp = state.build_response(false);
                    return Some(Action::UserPromptResponse(resp));
                }
            }
            _ => {}
        }
    }
    None
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw_user_prompt(f: &mut Frame, state: &mut UserPromptState) {
    let area = f.area();

    let dialog_w = 72u16.min(area.width.saturating_sub(4));
    let dialog_h = compute_height(state).min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(dialog_w)) / 2;
    let y = (area.height.saturating_sub(dialog_h)) / 2;
    let dialog = Rect::new(x, y, dialog_w, dialog_h);

    f.render_widget(Clear, dialog);

    let hint = match &state.phase {
        PromptPhase::Select { .. } => " ↑↓ nav · Enter select · Esc cancel ",
        PromptPhase::MultiSelect { .. } => " ↑↓ nav · Space toggle · a all · Enter submit · Esc cancel ",
        PromptPhase::Confirm { .. } => " ←→ toggle · y/n · Enter confirm · Esc cancel ",
        PromptPhase::TextInput { .. } => " Type response · Enter submit · Esc cancel ",
        PromptPhase::Form { .. } => " ↑↓/Tab nav · Type text · Enter next/submit · Esc cancel ",
    };

    let block = Block::default()
        .title(Span::styled(
            format!(" 💬 {} ", &state.prompt.title),
            Style::default()
                .fg(tp::ACCENT_BRIGHT)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(
            Line::from(Span::styled(hint, Style::default().fg(tp::MUTED))).right_aligned(),
        )
        .borders(Borders::ALL)
        .border_style(Style::default().fg(tp::ACCENT))
        .border_type(ratatui::widgets::BorderType::Rounded);

    let inner = block.inner(dialog);
    f.render_widget(block, dialog);

    // Description if present
    let (_desc_height, body_area) = if let Some(desc) = &state.prompt.description {
        let desc_lines = 2u16; // rough estimate
        let chunks = Layout::vertical([
            Constraint::Length(desc_lines),
            Constraint::Min(2),
        ])
        .split(inner);

        let desc_widget = Paragraph::new(Line::from(Span::styled(
            desc.clone(),
            Style::default().fg(tp::TEXT_DIM),
        )))
        .wrap(Wrap { trim: true });
        f.render_widget(desc_widget, chunks[0]);

        (desc_lines, chunks[1])
    } else {
        (0, inner)
    };

    match &mut state.phase {
        PromptPhase::Select { cursor, scroll } => {
            if let PromptType::Select { options, .. } = &state.prompt.prompt_type {
                draw_select_list(f, body_area, options, *cursor, scroll);
            }
        }
        PromptPhase::MultiSelect {
            cursor,
            scroll,
            selected,
        } => {
            if let PromptType::MultiSelect { options, .. } = &state.prompt.prompt_type {
                draw_multiselect_list(f, body_area, options, *cursor, scroll, selected);
            }
        }
        PromptPhase::Confirm { yes } => {
            draw_confirm(f, body_area, *yes);
        }
        PromptPhase::TextInput { input } => {
            let placeholder = if let PromptType::TextInput { placeholder, .. } = &state.prompt.prompt_type {
                placeholder.clone()
            } else {
                None
            };
            draw_text_input(f, body_area, input, placeholder.as_deref());
        }
        PromptPhase::Form { cursor, inputs } => {
            if let PromptType::Form { fields } = &state.prompt.prompt_type {
                draw_form(f, body_area, fields, *cursor, inputs);
            }
        }
    }
}

fn compute_height(state: &UserPromptState) -> u16 {
    let desc_lines: u16 = if state.prompt.description.is_some() { 3 } else { 0 };
    let body_lines: u16 = match &state.prompt.prompt_type {
        PromptType::Select { options, .. } => (options.len() as u16).min(16) + 1,
        PromptType::MultiSelect { options, .. } => (options.len() as u16).min(16) + 1,
        PromptType::Confirm { .. } => 3,
        PromptType::TextInput { .. } => 3,
        PromptType::Form { fields } => (fields.len() as u16 * 2) + 1,
    };
    // +2 for borders, +1 for title padding
    desc_lines + body_lines + 3
}

fn draw_select_list(
    f: &mut Frame,
    area: Rect,
    options: &[PromptOption],
    cursor: usize,
    scroll: &mut usize,
) {
    let visible = area.height as usize;
    if visible > 0 && options.len() > visible {
        if cursor >= *scroll + visible {
            *scroll = cursor - visible + 1;
        } else if cursor < *scroll {
            *scroll = cursor;
        }
    } else {
        *scroll = 0;
    }

    let items: Vec<ListItem> = options
        .iter()
        .enumerate()
        .skip(*scroll)
        .take(visible)
        .map(|(i, opt)| {
            let marker = if i == cursor { "❯ " } else { "  " };
            let style = if i == cursor {
                Style::default()
                    .fg(tp::ACCENT_BRIGHT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(tp::TEXT)
            };

            let mut spans = vec![
                Span::styled(marker, Style::default().fg(tp::ACCENT)),
                Span::styled(&opt.label, style),
            ];
            if let Some(desc) = &opt.description {
                spans.push(Span::styled(
                    format!("  {}", desc),
                    Style::default().fg(tp::TEXT_DIM),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    f.render_widget(List::new(items), area);
}

fn draw_multiselect_list(
    f: &mut Frame,
    area: Rect,
    options: &[PromptOption],
    cursor: usize,
    scroll: &mut usize,
    selected: &[bool],
) {
    let visible = area.height as usize;
    if visible > 0 && options.len() > visible {
        if cursor >= *scroll + visible {
            *scroll = cursor - visible + 1;
        } else if cursor < *scroll {
            *scroll = cursor;
        }
    } else {
        *scroll = 0;
    }

    let items: Vec<ListItem> = options
        .iter()
        .enumerate()
        .skip(*scroll)
        .take(visible)
        .map(|(i, opt)| {
            let marker = if i == cursor { "❯ " } else { "  " };
            let check = if selected.get(i).copied().unwrap_or(false) {
                "[✓] "
            } else {
                "[ ] "
            };
            let style = if i == cursor {
                Style::default()
                    .fg(tp::ACCENT_BRIGHT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(tp::TEXT)
            };

            let mut spans = vec![
                Span::styled(marker, Style::default().fg(tp::ACCENT)),
                Span::styled(check, Style::default().fg(tp::SUCCESS)),
                Span::styled(&opt.label, style),
            ];
            if let Some(desc) = &opt.description {
                spans.push(Span::styled(
                    format!("  {}", desc),
                    Style::default().fg(tp::TEXT_DIM),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    f.render_widget(List::new(items), area);
}

fn draw_confirm(f: &mut Frame, area: Rect, yes: bool) {
    let yes_style = if yes {
        Style::default()
            .fg(Color::Rgb(0x1E, 0x1C, 0x1A))
            .bg(tp::SUCCESS)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(tp::TEXT_DIM)
    };
    let no_style = if !yes {
        Style::default()
            .fg(Color::Rgb(0x1E, 0x1C, 0x1A))
            .bg(tp::ERROR)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(tp::TEXT_DIM)
    };

    let line = Line::from(vec![
        Span::raw("  "),
        Span::styled(" Yes ", yes_style),
        Span::raw("   "),
        Span::styled(" No ", no_style),
    ]);
    let centered_y = area.y + area.height / 2;
    let btn_area = Rect::new(area.x, centered_y, area.width, 1);
    f.render_widget(Paragraph::new(line), btn_area);
}

fn draw_text_input(f: &mut Frame, area: Rect, input: &str, placeholder: Option<&str>) {
    let display = if input.is_empty() {
        if let Some(ph) = placeholder {
            Line::from(Span::styled(
                format!(" > {}", ph),
                Style::default().fg(tp::MUTED),
            ))
        } else {
            Line::from(Span::styled(
                " > _",
                Style::default().fg(tp::TEXT),
            ))
        }
    } else {
        Line::from(vec![
            Span::styled(" > ", Style::default().fg(tp::ACCENT)),
            Span::styled(
                format!("{}_", input),
                Style::default().fg(tp::TEXT).add_modifier(Modifier::BOLD),
            ),
        ])
    };
    f.render_widget(Paragraph::new(display), area);
}

fn draw_form(
    f: &mut Frame,
    area: Rect,
    fields: &[FormField],
    cursor: usize,
    inputs: &[String],
) {
    let mut lines = Vec::new();
    for (i, field) in fields.iter().enumerate() {
        let is_active = i == cursor;
        let label_style = if is_active {
            Style::default()
                .fg(tp::ACCENT_BRIGHT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(tp::TEXT_DIM)
        };

        let required_marker = if field.required { " *" } else { "" };
        lines.push(Line::from(Span::styled(
            format!(" {}{}", field.label, required_marker),
            label_style,
        )));

        let val = inputs.get(i).map(|s| s.as_str()).unwrap_or("");
        let input_style = if is_active {
            Style::default().fg(tp::TEXT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(tp::TEXT)
        };
        let display_val = if val.is_empty() && is_active {
            format!("  > _")
        } else if val.is_empty() {
            if let Some(ph) = &field.placeholder {
                format!("  > {}", ph)
            } else {
                "  > ".to_string()
            }
        } else if is_active {
            format!("  > {}_", val)
        } else {
            format!("  > {}", val)
        };
        let val_style = if val.is_empty() && !is_active {
            Style::default().fg(tp::MUTED)
        } else {
            input_style
        };
        lines.push(Line::from(Span::styled(display_val, val_style)));
    }

    f.render_widget(Paragraph::new(lines), area);
}
