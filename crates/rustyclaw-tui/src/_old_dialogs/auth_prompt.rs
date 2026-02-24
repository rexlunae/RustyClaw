//! Gateway authentication prompt dialog (TOTP code entry).

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::action::Action;
use crate::panes::DisplayMessage;
use rustyclaw_core::types::GatewayStatus;
use crate::tui_palette as tp;

/// State for the gateway TOTP authentication prompt dialog.
pub struct AuthPromptState {
    /// Current input buffer (the 6-digit TOTP code being typed)
    pub input: String,
}

/// Handle key events when the gateway TOTP auth prompt is open.
/// Returns (updated state or None if closed, action to dispatch).
pub fn handle_auth_prompt_key(
    prompt: AuthPromptState,
    code: crossterm::event::KeyCode,
    messages: &mut Vec<DisplayMessage>,
    gateway_status: &mut GatewayStatus,
) -> (Option<AuthPromptState>, Action) {
    use crossterm::event::KeyCode;

    let mut prompt = prompt;

    match code {
        KeyCode::Esc => {
            messages.push(DisplayMessage::info("Authentication cancelled."));
            *gateway_status = GatewayStatus::Error;
            (None, Action::Update)
        }
        KeyCode::Char(c) if c.is_ascii_digit() && prompt.input.len() < 6 => {
            prompt.input.push(c);
            (Some(prompt), Action::Noop)
        }
        KeyCode::Backspace => {
            prompt.input.pop();
            (Some(prompt), Action::Noop)
        }
        KeyCode::Enter => {
            if prompt.input.len() == 6 {
                let code_str = prompt.input.clone();
                // Dialog is consumed — send the code
                (None, Action::GatewayAuthResponse(code_str))
            } else {
                (Some(prompt), Action::Noop)
            }
        }
        _ => (Some(prompt), Action::Noop),
    }
}

/// Draw a centered gateway TOTP auth prompt overlay.
pub fn draw_auth_prompt(frame: &mut ratatui::Frame<'_>, area: Rect, prompt: &AuthPromptState) {
    let dialog_w = 46.min(area.width.saturating_sub(4));
    let dialog_h = 7u16.min(area.height.saturating_sub(4)).max(5);
    let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
    let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

    frame.render_widget(Clear, dialog_area);

    let block = Block::default()
        .title(Span::styled(
            " 🔑 Gateway Authentication ",
            tp::title_focused(),
        ))
        .title_bottom(
            Line::from(Span::styled(
                " Esc to cancel ",
                Style::default().fg(tp::MUTED),
            ))
            .right_aligned(),
        )
        .borders(Borders::ALL)
        .border_style(tp::focused_border())
        .border_type(ratatui::widgets::BorderType::Rounded);

    let inner = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

    // Label
    if inner.height >= 1 {
        let label = Line::from(Span::styled(
            " Enter your 6-digit TOTP code:",
            Style::default().fg(tp::TEXT),
        ));
        frame.render_widget(
            Paragraph::new(label),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
    }

    // Input with dots for each digit and placeholders
    if inner.height >= 3 {
        let input_area = Rect::new(inner.x + 1, inner.y + 2, inner.width.saturating_sub(2), 1);
        let mut spans = vec![Span::styled("❯ ", Style::default().fg(tp::ACCENT))];
        for (i, ch) in prompt.input.chars().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" ", Style::default()));
            }
            spans.push(Span::styled(
                ch.to_string(),
                Style::default()
                    .fg(tp::ACCENT_BRIGHT)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        for i in prompt.input.len()..6 {
            if i > 0 {
                spans.push(Span::styled(" ", Style::default()));
            }
            spans.push(Span::styled("·", Style::default().fg(tp::MUTED)));
        }
        let line = Line::from(spans);
        frame.render_widget(Paragraph::new(line), input_area);

        // Cursor
        let cursor_x = input_area.x + 2 + (prompt.input.len() * 2) as u16;
        frame.set_cursor_position((cursor_x, input_area.y));
    }
}
