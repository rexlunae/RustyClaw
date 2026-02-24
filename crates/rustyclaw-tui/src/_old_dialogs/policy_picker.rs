//! Access policy picker dialog.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

use crate::action::Action;
use crate::panes::DisplayMessage;
use rustyclaw_core::secrets::{AccessPolicy, SecretsManager};
use crate::tui_palette as tp;

/// Which policy option is highlighted in the policy-picker dialog.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PolicyPickerOption {
    Open,
    Ask,
    Auth,
    Skill,
}

/// Phase of the SKILL-policy sub-flow inside the policy picker.
#[derive(Debug, Clone, PartialEq)]
pub enum PolicyPickerPhase {
    /// Selecting among OPEN / ASK / AUTH / SKILL
    Selecting,
    /// Editing the skill name list (comma-separated)
    EditingSkills { input: String },
}

/// State for the access-policy picker dialog overlay.
pub struct PolicyPickerState {
    /// Vault key name of the credential
    pub cred_name: String,
    /// Currently highlighted policy option
    pub selected: PolicyPickerOption,
    /// Current dialog phase
    pub phase: PolicyPickerPhase,
}

/// Handle key events when the policy-picker dialog is open.
/// Returns (updated state or None if closed, action to dispatch).
pub fn handle_policy_picker_key(
    picker: PolicyPickerState,
    code: crossterm::event::KeyCode,
    secrets_manager: &mut SecretsManager,
    messages: &mut Vec<DisplayMessage>,
) -> (Option<PolicyPickerState>, Action) {
    use crossterm::event::KeyCode;

    let mut picker = picker;

    match picker.phase {
        PolicyPickerPhase::Selecting => {
            let options = [
                PolicyPickerOption::Open,
                PolicyPickerOption::Ask,
                PolicyPickerOption::Auth,
                PolicyPickerOption::Skill,
            ];

            match code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    // Cancel — go back to credential dialog
                    (None, Action::Noop)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    let cur = options.iter().position(|o| *o == picker.selected).unwrap_or(0);
                    let next = if cur == 0 { options.len() - 1 } else { cur - 1 };
                    picker.selected = options[next];
                    (Some(picker), Action::Noop)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let cur = options.iter().position(|o| *o == picker.selected).unwrap_or(0);
                    let next = (cur + 1) % options.len();
                    picker.selected = options[next];
                    (Some(picker), Action::Noop)
                }
                KeyCode::Enter => match picker.selected {
                    PolicyPickerOption::Open => {
                        match secrets_manager.set_credential_policy(
                            &picker.cred_name,
                            AccessPolicy::Always,
                        ) {
                            Ok(()) => {
                                messages.push(DisplayMessage::success(format!(
                                    "Policy for '{}' set to OPEN.",
                                    picker.cred_name,
                                )));
                            }
                            Err(e) => {
                                messages.push(DisplayMessage::error(format!(
                                    "Failed to set policy: {}",
                                    e,
                                )));
                            }
                        }
                        (None, Action::Update)
                    }
                    PolicyPickerOption::Ask => {
                        match secrets_manager.set_credential_policy(
                            &picker.cred_name,
                            AccessPolicy::WithApproval,
                        ) {
                            Ok(()) => {
                                messages.push(DisplayMessage::success(format!(
                                    "Policy for '{}' set to ASK.",
                                    picker.cred_name,
                                )));
                            }
                            Err(e) => {
                                messages.push(DisplayMessage::error(format!(
                                    "Failed to set policy: {}",
                                    e,
                                )));
                            }
                        }
                        (None, Action::Update)
                    }
                    PolicyPickerOption::Auth => {
                        match secrets_manager.set_credential_policy(
                            &picker.cred_name,
                            AccessPolicy::WithAuth,
                        ) {
                            Ok(()) => {
                                messages.push(DisplayMessage::success(format!(
                                    "Policy for '{}' set to AUTH.",
                                    picker.cred_name,
                                )));
                            }
                            Err(e) => {
                                messages.push(DisplayMessage::error(format!(
                                    "Failed to set policy: {}",
                                    e,
                                )));
                            }
                        }
                        (None, Action::Update)
                    }
                    PolicyPickerOption::Skill => {
                        // Transition to skill name input phase
                        picker.phase = PolicyPickerPhase::EditingSkills {
                            input: String::new(),
                        };
                        (Some(picker), Action::Noop)
                    }
                },
                _ => (Some(picker), Action::Noop),
            }
        }
        PolicyPickerPhase::EditingSkills { ref mut input } => match code {
            crossterm::event::KeyCode::Esc => {
                // Go back to the selection phase
                picker.phase = PolicyPickerPhase::Selecting;
                (Some(picker), Action::Noop)
            }
            crossterm::event::KeyCode::Char(c) => {
                input.push(c);
                (Some(picker), Action::Noop)
            }
            crossterm::event::KeyCode::Backspace => {
                input.pop();
                (Some(picker), Action::Noop)
            }
            crossterm::event::KeyCode::Enter => {
                let skills: Vec<String> = input
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();

                match secrets_manager.set_credential_policy(
                    &picker.cred_name,
                    AccessPolicy::SkillOnly(skills.clone()),
                ) {
                    Ok(()) => {
                        if skills.is_empty() {
                            messages.push(DisplayMessage::success(format!(
                                "Policy for '{}' set to SKILL (locked — no skills).",
                                picker.cred_name,
                            )));
                        } else {
                            messages.push(DisplayMessage::success(format!(
                                "Policy for '{}' set to SKILL ({}).",
                                picker.cred_name,
                                skills.join(", "),
                            )));
                        }
                    }
                    Err(e) => {
                        messages.push(DisplayMessage::error(format!(
                            "Failed to set policy: {}",
                            e,
                        )));
                    }
                }
                (None, Action::Update)
            }
            _ => (Some(picker), Action::Noop),
        },
    }
}


/// Handle key events for the policy picker (gateway-backed).
/// Returns Actions that trigger gateway sends instead of calling SecretsManager directly.
pub fn handle_policy_picker_key_gateway(
    picker: PolicyPickerState,
    code: crossterm::event::KeyCode,
    messages: &mut Vec<DisplayMessage>,
) -> (Option<PolicyPickerState>, Action) {
    use crossterm::event::KeyCode;

    let mut picker = picker;

    match picker.phase {
        PolicyPickerPhase::Selecting => {
            let options = [
                PolicyPickerOption::Open,
                PolicyPickerOption::Ask,
                PolicyPickerOption::Auth,
                PolicyPickerOption::Skill,
            ];

            match code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    (None, Action::Noop)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    let cur = options.iter().position(|o| *o == picker.selected).unwrap_or(0);
                    let next = if cur == 0 { options.len() - 1 } else { cur - 1 };
                    picker.selected = options[next];
                    (Some(picker), Action::Noop)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let cur = options.iter().position(|o| *o == picker.selected).unwrap_or(0);
                    let next = (cur + 1) % options.len();
                    picker.selected = options[next];
                    (Some(picker), Action::Noop)
                }
                KeyCode::Enter => match picker.selected {
                    PolicyPickerOption::Open => {
                        let frame = serde_json::json!({
                            "type": "secrets_set_policy",
                            "name": picker.cred_name,
                            "policy": "always",
                        });
                        messages.push(DisplayMessage::info(format!(
                            "Setting policy for '{}' to OPEN…", picker.cred_name,
                        )));
                        (None, Action::SendToGateway(frame.to_string()))
                    }
                    PolicyPickerOption::Ask => {
                        let frame = serde_json::json!({
                            "type": "secrets_set_policy",
                            "name": picker.cred_name,
                            "policy": "ask",
                        });
                        messages.push(DisplayMessage::info(format!(
                            "Setting policy for '{}' to ASK…", picker.cred_name,
                        )));
                        (None, Action::SendToGateway(frame.to_string()))
                    }
                    PolicyPickerOption::Auth => {
                        let frame = serde_json::json!({
                            "type": "secrets_set_policy",
                            "name": picker.cred_name,
                            "policy": "auth",
                        });
                        messages.push(DisplayMessage::info(format!(
                            "Setting policy for '{}' to AUTH…", picker.cred_name,
                        )));
                        (None, Action::SendToGateway(frame.to_string()))
                    }
                    PolicyPickerOption::Skill => {
                        picker.phase = PolicyPickerPhase::EditingSkills {
                            input: String::new(),
                        };
                        (Some(picker), Action::Noop)
                    }
                },
                _ => (Some(picker), Action::Noop),
            }
        }
        PolicyPickerPhase::EditingSkills { ref mut input } => match code {
            crossterm::event::KeyCode::Esc => {
                picker.phase = PolicyPickerPhase::Selecting;
                (Some(picker), Action::Noop)
            }
            crossterm::event::KeyCode::Char(c) => {
                input.push(c);
                (Some(picker), Action::Noop)
            }
            crossterm::event::KeyCode::Backspace => {
                input.pop();
                (Some(picker), Action::Noop)
            }
            crossterm::event::KeyCode::Enter => {
                let skills: Vec<String> = input
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();

                let frame = serde_json::json!({
                    "type": "secrets_set_policy",
                    "name": picker.cred_name,
                    "policy": "skill_only",
                    "skills": skills,
                });
                messages.push(DisplayMessage::info(format!(
                    "Setting policy for '{}' to SKILL…", picker.cred_name,
                )));
                (None, Action::SendToGateway(frame.to_string()))
            }
            _ => (Some(picker), Action::Noop),
        },
    }
}

/// Draw a centered policy-picker dialog overlay.
pub fn draw_policy_picker(frame: &mut ratatui::Frame<'_>, area: Rect, picker: &PolicyPickerState) {
    let dialog_w = 52.min(area.width.saturating_sub(4));
    let dialog_h = match picker.phase {
        PolicyPickerPhase::Selecting => 12u16,
        PolicyPickerPhase::EditingSkills { .. } => 8u16,
    }
    .min(area.height.saturating_sub(4))
    .max(8);
    let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
    let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

    frame.render_widget(Clear, dialog_area);

    match picker.phase {
        PolicyPickerPhase::Selecting => {
            let title = format!(" {} — Access Policy ", picker.cred_name);
            let hint = " ↑↓ navigate · Enter select · Esc cancel ";

            let block = Block::default()
                .title(Span::styled(&title, tp::title_focused()))
                .title_bottom(
                    Line::from(Span::styled(hint, Style::default().fg(tp::MUTED))).right_aligned(),
                )
                .borders(Borders::ALL)
                .border_style(tp::focused_border())
                .border_type(ratatui::widgets::BorderType::Rounded);

            let inner = block.inner(dialog_area);
            frame.render_widget(block, dialog_area);

            let options: Vec<(PolicyPickerOption, &str, &str, Color)> = vec![
                (
                    PolicyPickerOption::Open,
                    "OPEN",
                    "  Agent can read anytime",
                    tp::SUCCESS,
                ),
                (
                    PolicyPickerOption::Ask,
                    "ASK",
                    "   Agent asks per use",
                    tp::WARN,
                ),
                (
                    PolicyPickerOption::Auth,
                    "AUTH",
                    "  Re-authenticate each time",
                    tp::ERROR,
                ),
                (
                    PolicyPickerOption::Skill,
                    "SKILL",
                    " Only named skills may read",
                    tp::INFO,
                ),
            ];

            let items: Vec<ListItem> = options
                .iter()
                .map(|(opt, badge_text, desc, badge_color)| {
                    let is_selected = *opt == picker.selected;
                    let marker = if is_selected { "❯ " } else { "  " };

                    let badge = Span::styled(
                        format!(" {} ", badge_text),
                        Style::default()
                            .fg(Color::Rgb(0x1E, 0x1C, 0x1A))
                            .bg(*badge_color),
                    );

                    let desc_style = if is_selected {
                        Style::default()
                            .fg(tp::ACCENT_BRIGHT)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(tp::TEXT_DIM)
                    };

                    ListItem::new(Line::from(vec![
                        Span::styled(marker, Style::default().fg(tp::ACCENT)),
                        badge,
                        Span::styled(*desc, desc_style),
                    ]))
                })
                .collect();

            // Render with a blank row at top for padding
            let mut all_items = vec![ListItem::new("")];
            all_items.extend(items);

            let list = List::new(all_items);
            frame.render_widget(list, inner);
        }
        PolicyPickerPhase::EditingSkills { ref input } => {
            let title = format!(" {} — SKILL Policy ", picker.cred_name);
            let hint = " Enter confirm · Esc back ";

            let block = Block::default()
                .title(Span::styled(&title, tp::title_focused()))
                .title_bottom(
                    Line::from(Span::styled(hint, Style::default().fg(tp::MUTED))).right_aligned(),
                )
                .borders(Borders::ALL)
                .border_style(tp::focused_border())
                .border_type(ratatui::widgets::BorderType::Rounded);

            let inner = block.inner(dialog_area);
            frame.render_widget(block, dialog_area);

            let prompt_text = vec![
                Line::from(""),
                Line::from(Span::styled(
                    " Enter skill names (comma-separated):",
                    Style::default().fg(tp::TEXT_DIM),
                )),
                Line::from(Span::styled(
                    " Leave empty to lock the credential.",
                    Style::default().fg(tp::MUTED),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(" > ", Style::default().fg(tp::ACCENT)),
                    Span::styled(
                        format!("{}_", input),
                        Style::default().fg(tp::TEXT).add_modifier(Modifier::BOLD),
                    ),
                ]),
            ];

            let paragraph = Paragraph::new(prompt_text);
            frame.render_widget(paragraph, inner);
        }
    }
}
