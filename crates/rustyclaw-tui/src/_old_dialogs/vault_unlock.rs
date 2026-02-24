//! Gateway vault unlock prompt dialog (password entry).

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::action::Action;
use crate::panes::DisplayMessage;
use crate::tui_palette as tp;

/// State for the gateway vault unlock prompt dialog.
pub struct VaultUnlockPromptState {
    /// Current input buffer (the vault password being typed)
    pub input: String,
}

/// Handle key events when the vault unlock password prompt is open.
/// Returns (updated state or None if closed, action to dispatch).
pub fn handle_vault_unlock_prompt_key(
    prompt: VaultUnlockPromptState,
    code: crossterm::event::KeyCode,
    messages: &mut Vec<DisplayMessage>,
) -> (Option<VaultUnlockPromptState>, Action) {
    use crossterm::event::KeyCode;

    let mut prompt = prompt;

    match code {
        KeyCode::Esc => {
            messages.push(DisplayMessage::info("Vault unlock cancelled."));
            (None, Action::Update)
        }
        KeyCode::Enter => {
            if prompt.input.is_empty() {
                (Some(prompt), Action::Noop)
            } else {
                let password = prompt.input.clone();
                // Dialog is consumed — send the password
                (None, Action::GatewayUnlockVault(password))
            }
        }
        KeyCode::Backspace => {
            prompt.input.pop();
            (Some(prompt), Action::Noop)
        }
        KeyCode::Char(c) => {
            prompt.input.push(c);
            (Some(prompt), Action::Noop)
        }
        _ => (Some(prompt), Action::Noop),
    }
}

/// Draw a centered vault unlock password prompt overlay.
pub fn draw_vault_unlock_prompt(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    prompt: &VaultUnlockPromptState,
) {
    let dialog_w = 52.min(area.width.saturating_sub(4));
    let dialog_h = 7u16.min(area.height.saturating_sub(4)).max(5);
    let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
    let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

    frame.render_widget(Clear, dialog_area);

    let block = Block::default()
        .title(Span::styled(
            " 🔒 Unlock Gateway Vault ",
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
            " Enter vault password:",
            Style::default().fg(tp::TEXT),
        ));
        frame.render_widget(
            Paragraph::new(label),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
    }

    // Masked input
    if inner.height >= 3 {
        let input_area = Rect::new(inner.x + 1, inner.y + 2, inner.width.saturating_sub(2), 1);
        let masked: String = "•".repeat(prompt.input.len());
        let line = Line::from(vec![
            Span::styled("❯ ", Style::default().fg(tp::ACCENT)),
            Span::styled(&masked, Style::default().fg(tp::TEXT)),
        ]);
        frame.render_widget(Paragraph::new(line), input_area);

        // Cursor
        frame.set_cursor_position((input_area.x + 2 + masked.len() as u16, input_area.y));
    }
}
