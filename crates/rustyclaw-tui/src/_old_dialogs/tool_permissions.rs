//! Tool permissions editor dialog.
//!
//! Allows the user to set per-tool permission levels:
//! Allow, Deny, Ask, or SkillOnly.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

use crate::action::Action;
use crate::panes::DisplayMessage;
use crate::tui_palette as tp;
use rustyclaw_core::tools::{self, ToolPermission};
use std::collections::HashMap;

/// Phase of the tool permissions dialog.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolPermissionsPhase {
    /// Browsing / selecting tool permissions
    Selecting,
    /// Editing the skill name list for SkillOnly mode
    EditingSkills { input: String },
}

/// State for the tool permissions dialog overlay.
pub struct ToolPermissionsState {
    /// All tool names (sorted)
    pub tool_names: Vec<&'static str>,
    /// Currently selected index into tool_names
    pub selected: usize,
    /// Working copy of permissions (committed on save)
    pub permissions: HashMap<String, ToolPermission>,
    /// Current dialog phase
    pub phase: ToolPermissionsPhase,
    /// Vertical scroll offset for the list
    pub scroll_offset: usize,
    /// Whether any changes were made
    pub dirty: bool,
}

impl ToolPermissionsState {
    pub fn new(existing: &HashMap<String, ToolPermission>) -> Self {
        let tool_names = tools::all_tool_names();
        let permissions = existing.clone();
        Self {
            tool_names,
            selected: 0,
            permissions,
            phase: ToolPermissionsPhase::Selecting,
            scroll_offset: 0,
            dirty: false,
        }
    }

    /// Get the permission for the currently selected tool.
    pub fn current_permission(&self) -> ToolPermission {
        if let Some(name) = self.tool_names.get(self.selected) {
            self.permissions
                .get(*name)
                .cloned()
                .unwrap_or_default()
        } else {
            ToolPermission::default()
        }
    }

    /// Get the name of the currently selected tool.
    pub fn current_tool(&self) -> &str {
        self.tool_names
            .get(self.selected)
            .copied()
            .unwrap_or("unknown")
    }
}

/// Handle key events when the tool-permissions dialog is open.
/// Returns (updated state or None if closed, action to dispatch).
pub fn handle_tool_permissions_key(
    state: ToolPermissionsState,
    code: crossterm::event::KeyCode,
    messages: &mut Vec<DisplayMessage>,
) -> (Option<ToolPermissionsState>, Action) {
    use crossterm::event::KeyCode;

    let mut state = state;

    match state.phase {
        ToolPermissionsPhase::Selecting => match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                if state.dirty {
                    messages.push(DisplayMessage::success(
                        "Tool permissions saved.".to_string(),
                    ));
                    // Return the action to save — the app handler will persist
                    return (None, Action::SaveToolPermissions(state.permissions));
                }
                (None, Action::Update)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if state.selected > 0 {
                    state.selected -= 1;
                    // Scroll up if needed
                    if state.selected < state.scroll_offset {
                        state.scroll_offset = state.selected;
                    }
                }
                (Some(state), Action::Noop)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if state.selected + 1 < state.tool_names.len() {
                    state.selected += 1;
                }
                (Some(state), Action::Noop)
            }
            KeyCode::Home => {
                state.selected = 0;
                state.scroll_offset = 0;
                (Some(state), Action::Noop)
            }
            KeyCode::End => {
                state.selected = state.tool_names.len().saturating_sub(1);
                (Some(state), Action::Noop)
            }
            KeyCode::PageUp => {
                state.selected = state.selected.saturating_sub(15);
                if state.selected < state.scroll_offset {
                    state.scroll_offset = state.selected;
                }
                (Some(state), Action::Noop)
            }
            KeyCode::PageDown => {
                state.selected = (state.selected + 15).min(state.tool_names.len().saturating_sub(1));
                (Some(state), Action::Noop)
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                // Cycle permission for the selected tool
                let name = state.current_tool().to_string();
                let current = state.current_permission();
                let next = current.cycle();

                // If cycling into SkillOnly, open the skill name editor
                if matches!(next, ToolPermission::SkillOnly(_)) {
                    let existing_skills = if let ToolPermission::SkillOnly(ref s) = current {
                        s.join(", ")
                    } else {
                        String::new()
                    };
                    state.permissions.insert(name, ToolPermission::SkillOnly(Vec::new()));
                    state.dirty = true;
                    state.phase = ToolPermissionsPhase::EditingSkills {
                        input: existing_skills,
                    };
                } else {
                    state.permissions.insert(name, next);
                    state.dirty = true;
                }
                (Some(state), Action::Noop)
            }
            KeyCode::Char('a') => {
                // Quick-set: Allow
                let name = state.current_tool().to_string();
                state.permissions.insert(name, ToolPermission::Allow);
                state.dirty = true;
                (Some(state), Action::Noop)
            }
            KeyCode::Char('d') => {
                // Quick-set: Deny
                let name = state.current_tool().to_string();
                state.permissions.insert(name, ToolPermission::Deny);
                state.dirty = true;
                (Some(state), Action::Noop)
            }
            KeyCode::Char('?') => {
                // Quick-set: Ask
                let name = state.current_tool().to_string();
                state.permissions.insert(name, ToolPermission::Ask);
                state.dirty = true;
                (Some(state), Action::Noop)
            }
            KeyCode::Char('s') => {
                // Quick-set: SkillOnly (open editor)
                let name = state.current_tool().to_string();
                let existing_skills = if let Some(ToolPermission::SkillOnly(s)) =
                    state.permissions.get(&name)
                {
                    s.join(", ")
                } else {
                    String::new()
                };
                state.permissions.insert(name.clone(), ToolPermission::SkillOnly(Vec::new()));
                state.dirty = true;
                state.phase = ToolPermissionsPhase::EditingSkills {
                    input: existing_skills,
                };
                (Some(state), Action::Noop)
            }
            KeyCode::Char('A') => {
                // Set ALL tools to Allow
                for name in &state.tool_names {
                    state.permissions.remove(*name);
                }
                state.dirty = true;
                messages.push(DisplayMessage::info("All tools set to ALLOW."));
                (Some(state), Action::Noop)
            }
            KeyCode::Char('D') => {
                // Set ALL tools to Deny
                for name in &state.tool_names {
                    state
                        .permissions
                        .insert(name.to_string(), ToolPermission::Deny);
                }
                state.dirty = true;
                messages.push(DisplayMessage::info("All tools set to DENY."));
                (Some(state), Action::Noop)
            }
            _ => (Some(state), Action::Noop),
        },
        ToolPermissionsPhase::EditingSkills { ref mut input } => match code {
            crossterm::event::KeyCode::Esc => {
                state.phase = ToolPermissionsPhase::Selecting;
                (Some(state), Action::Noop)
            }
            crossterm::event::KeyCode::Char(c) => {
                input.push(c);
                (Some(state), Action::Noop)
            }
            crossterm::event::KeyCode::Backspace => {
                input.pop();
                (Some(state), Action::Noop)
            }
            crossterm::event::KeyCode::Enter => {
                let skills: Vec<String> = input
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();

                let name = state.current_tool().to_string();
                state
                    .permissions
                    .insert(name, ToolPermission::SkillOnly(skills));
                state.dirty = true;
                state.phase = ToolPermissionsPhase::Selecting;
                (Some(state), Action::Noop)
            }
            _ => (Some(state), Action::Noop),
        },
    }
}

/// Draw the tool permissions dialog overlay.
pub fn draw_tool_permissions(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    state: &mut ToolPermissionsState,
) {
    // Width: fit marker(2) + name(22) + badge(12) + description(~40) + padding
    let dialog_w = 110u16.min(area.width.saturating_sub(2));
    let dialog_h = match state.phase {
        ToolPermissionsPhase::Selecting => {
            // +4 for borders + header, +3 for separator + description area
            let ideal = state.tool_names.len() as u16 + 7;
            ideal.min(area.height.saturating_sub(2)).max(14)
        }
        ToolPermissionsPhase::EditingSkills { .. } => 12u16.min(area.height.saturating_sub(2)),
    };
    let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
    let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

    frame.render_widget(Clear, dialog_area);

    match state.phase {
        ToolPermissionsPhase::Selecting => {
            let title = " Tool Permissions ";
            let hint =
                " ↑↓ nav · Enter/Space cycle · a/d/?/s quick-set · A/D all · Esc close ";

            let block = Block::default()
                .title(Span::styled(title, tp::title_focused()))
                .title_bottom(
                    Line::from(Span::styled(hint, Style::default().fg(tp::MUTED))).right_aligned(),
                )
                .borders(Borders::ALL)
                .border_style(tp::focused_border())
                .border_type(ratatui::widgets::BorderType::Rounded);

            let inner = block.inner(dialog_area);
            frame.render_widget(block, dialog_area);

            // Split inner into: tool list (flexible) + separator + description (2 rows)
            let chunks = Layout::vertical([
                Constraint::Min(4),    // tool list
                Constraint::Length(3), // separator + 2 description lines
            ])
            .split(inner);

            let list_area = chunks[0];
            let desc_area = chunks[1];

            // Calculate visible range with stable scroll tracking
            let visible_rows = list_area.height.saturating_sub(1) as usize; // -1 for header
            let scroll = if visible_rows >= state.tool_names.len() {
                // Everything fits — no scrolling needed
                0
            } else if state.selected >= state.scroll_offset + visible_rows {
                // Cursor moved below viewport — scroll down
                state.selected - visible_rows + 1
            } else if state.selected < state.scroll_offset {
                // Cursor moved above viewport — scroll up
                state.selected
            } else {
                state.scroll_offset
            };
            state.scroll_offset = scroll;

            // Header row
            let header = ListItem::new(Line::from(vec![
                Span::styled(
                    format!("  {:<22} ", "TOOL"),
                    Style::default()
                        .fg(tp::ACCENT_BRIGHT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:<12}", "PERMISSION"),
                    Style::default()
                        .fg(tp::ACCENT_BRIGHT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "DESCRIPTION",
                    Style::default()
                        .fg(tp::ACCENT_BRIGHT)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));

            let mut items: Vec<ListItem> = vec![header];

            for (i, name) in state
                .tool_names
                .iter()
                .enumerate()
                .skip(scroll)
                .take(visible_rows)
            {
                let perm = state
                    .permissions
                    .get(*name)
                    .cloned()
                    .unwrap_or_default();
                let is_selected = i == state.selected;
                let marker = if is_selected { "❯ " } else { "  " };

                let badge_text = format!(" {} ", perm.badge());
                let badge_color = match perm {
                    ToolPermission::Allow => tp::SUCCESS,
                    ToolPermission::Ask => tp::WARN,
                    ToolPermission::Deny => tp::ERROR,
                    ToolPermission::SkillOnly(_) => tp::INFO,
                };

                let name_style = if is_selected {
                    Style::default()
                        .fg(tp::ACCENT_BRIGHT)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(tp::TEXT)
                };

                // Truncate tool name if needed, pad to align
                let display_name = format!("{:<22}", name);

                // badge_text is e.g. " Allow ", " Deny ", " Ask ", " Skill "
                let badge_len = badge_text.len();
                // Pad after the badge so the description column aligns
                let pad = if badge_len < 12 {
                    " ".repeat(12 - badge_len)
                } else {
                    " ".to_string()
                };

                let mut spans = vec![
                    Span::styled(marker, Style::default().fg(tp::ACCENT)),
                    Span::styled(display_name, name_style),
                    Span::styled(
                        badge_text,
                        Style::default()
                            .fg(Color::Rgb(0x1E, 0x1C, 0x1A))
                            .bg(badge_color),
                    ),
                    Span::raw(pad),
                ];

                // Show skill names for SkillOnly inline after badge
                if let ToolPermission::SkillOnly(ref skills) = perm {
                    if !skills.is_empty() {
                        spans.push(Span::styled(
                            format!("({})", skills.join(", ")),
                            Style::default().fg(tp::TEXT_DIM),
                        ));
                    }
                }

                // Tool description column
                spans.push(Span::styled(
                    tools::tool_summary(name),
                    Style::default().fg(tp::TEXT_DIM),
                ));

                items.push(ListItem::new(Line::from(spans)));
            }

            let list = List::new(items);
            frame.render_widget(list, list_area);

            // ── Permission description for the selected tool ────────
            let current_perm = state.current_permission();
            let desc_lines = vec![
                Line::from(Span::styled(
                    "─".repeat(desc_area.width as usize),
                    Style::default().fg(tp::SURFACE_7),
                )),
                Line::from(vec![
                    Span::styled(
                        format!(" {} ", current_perm.badge()),
                        Style::default()
                            .fg(Color::Rgb(0x1E, 0x1C, 0x1A))
                            .bg(match current_perm {
                                ToolPermission::Allow => tp::SUCCESS,
                                ToolPermission::Ask => tp::WARN,
                                ToolPermission::Deny => tp::ERROR,
                                ToolPermission::SkillOnly(_) => tp::INFO,
                            }),
                    ),
                    Span::styled(
                        format!(" {}", current_perm.description()),
                        Style::default().fg(tp::TEXT_DIM),
                    ),
                ]),
            ];
            frame.render_widget(Paragraph::new(desc_lines), desc_area);

            // Scroll indicator when list is longer than viewport
            let total = state.tool_names.len();
            if total > visible_rows {
                let indicator = format!(
                    " {}-{} of {} ",
                    scroll + 1,
                    (scroll + visible_rows).min(total),
                    total,
                );
                let ind_w = indicator.len() as u16;
                let ind_area = Rect::new(
                    dialog_area.right().saturating_sub(ind_w + 2),
                    dialog_area.y,
                    ind_w,
                    1,
                );
                frame.render_widget(
                    Paragraph::new(Span::styled(indicator, Style::default().fg(tp::TEXT_DIM))),
                    ind_area,
                );
            }
        }
        ToolPermissionsPhase::EditingSkills { ref input } => {
            let tool = state.current_tool();
            let title = format!(" {} — Skill-Only Access ", tool);
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
                    " Only these skills can invoke this tool.",
                    Style::default().fg(tp::MUTED),
                )),
                Line::from(Span::styled(
                    " Leave empty to block all skill access.",
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
