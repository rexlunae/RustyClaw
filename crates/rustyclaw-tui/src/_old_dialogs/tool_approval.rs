//! Tool approval confirmation dialog.
//!
//! Shown when a tool has Ask permission — lets the user approve or deny
//! the specific tool invocation.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::action::Action;
use crate::tui_palette as tp;

/// State for the tool approval confirmation dialog.
pub struct ToolApprovalState {
    /// The tool call ID (needed to send the response).
    pub id: String,
    /// The tool name.
    pub name: String,
    /// The tool call arguments (JSON).
    pub arguments: serde_json::Value,
    /// Currently highlighted button: true = Allow, false = Deny.
    pub selected_allow: bool,
}

impl ToolApprovalState {
    pub fn new(id: String, name: String, arguments: serde_json::Value) -> Self {
        Self {
            id,
            name,
            arguments,
            selected_allow: true,
        }
    }
}

/// Handle a key event for the tool approval dialog.
/// Returns Some(Action) if the dialog should close with a response.
pub fn handle_tool_approval_key(
    state: &mut ToolApprovalState,
    key: KeyEvent,
) -> Option<Action> {
    match key.code {
        KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::Char('h') | KeyCode::Char('l') => {
            state.selected_allow = !state.selected_allow;
            None
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            Some(Action::ToolApprovalResponse {
                id: state.id.clone(),
                approved: true,
            })
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            Some(Action::ToolApprovalResponse {
                id: state.id.clone(),
                approved: false,
            })
        }
        KeyCode::Enter => {
            Some(Action::ToolApprovalResponse {
                id: state.id.clone(),
                approved: state.selected_allow,
            })
        }
        _ => None,
    }
}

/// Draw the tool approval dialog as a centered overlay.
pub fn draw_tool_approval(f: &mut Frame, state: &ToolApprovalState) {
    let area = f.area();

    // Center dialog — 60 wide, 14 tall
    let width = 60u16.min(area.width.saturating_sub(4));
    let height = 14u16.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let dialog = Rect::new(x, y, width, height);

    f.render_widget(Clear, dialog);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(tp::WARN))
        .title(" ⚠ Tool Approval Required ");

    let inner = block.inner(dialog);
    f.render_widget(block, dialog);

    let chunks = Layout::vertical([
        Constraint::Length(2), // Tool name
        Constraint::Length(1), // Separator
        Constraint::Min(4),   // Arguments
        Constraint::Length(1), // Separator
        Constraint::Length(1), // Buttons
        Constraint::Length(1), // Hint
    ])
    .split(inner);

    // Tool name
    let name_line = Line::from(vec![
        Span::styled("Tool: ", Style::default().fg(Color::Gray)),
        Span::styled(&state.name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
    ]);
    f.render_widget(Paragraph::new(vec![
        name_line,
        Line::from(Span::styled(
            "This tool requires your approval to run.",
            Style::default().fg(Color::Yellow),
        )),
    ]), chunks[0]);

    // Arguments preview
    let args_str = serde_json::to_string_pretty(&state.arguments)
        .unwrap_or_else(|_| state.arguments.to_string());
    let args_display = if args_str.len() > 300 {
        format!("{}…", &args_str[..300])
    } else {
        args_str
    };
    let args_para = Paragraph::new(args_display)
        .style(Style::default().fg(Color::DarkGray))
        .wrap(Wrap { trim: true });
    f.render_widget(args_para, chunks[2]);

    // Buttons
    let allow_style = if state.selected_allow {
        Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Green)
    };
    let deny_style = if !state.selected_allow {
        Style::default().fg(Color::Black).bg(Color::Red).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red)
    };

    let buttons = Line::from(vec![
        Span::raw("  "),
        Span::styled(" ✓ Allow ", allow_style),
        Span::raw("   "),
        Span::styled(" ✗ Deny ", deny_style),
    ]);
    f.render_widget(Paragraph::new(buttons), chunks[4]);

    // Hint
    let hint = Line::from(Span::styled(
        " y=allow  n/Esc=deny  ←/→=select  Enter=confirm",
        Style::default().fg(Color::DarkGray),
    ));
    f.render_widget(Paragraph::new(hint), chunks[5]);
}
