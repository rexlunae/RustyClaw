//! TOTP (2FA) setup dialog.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::action::Action;
use rustyclaw_core::config::Config;
use crate::panes::DisplayMessage;
use rustyclaw_core::secrets::SecretsManager;
use crate::tui_palette as tp;

/// Phase of the TOTP setup dialog.
#[derive(Debug, Clone, PartialEq)]
pub enum TotpDialogPhase {
    /// Show the otpauth URL and ask user to enter TOTP code to verify
    ShowUri { uri: String, input: String },
    /// TOTP is already set up — offer to remove it
    AlreadyConfigured,
    /// Verification succeeded
    Verified,
    /// Verification failed — let the user retry
    Failed { uri: String, input: String },
}

/// State for the 2FA (TOTP) setup dialog overlay.
pub struct TotpDialogState {
    pub phase: TotpDialogPhase,
}

/// Handle key events when the TOTP dialog is open.
/// Returns (updated state or None if closed, action to dispatch).
pub fn handle_totp_dialog_key(
    dlg: TotpDialogState,
    code: crossterm::event::KeyCode,
    secrets_manager: &mut SecretsManager,
    config: &mut Config,
    messages: &mut Vec<DisplayMessage>,
) -> (Option<TotpDialogState>, Action) {
    use crossterm::event::KeyCode;

    let mut dlg = dlg;

    match dlg.phase {
        TotpDialogPhase::ShowUri {
            ref mut uri,
            ref mut input,
        }
        | TotpDialogPhase::Failed {
            ref mut uri,
            ref mut input,
        } => match code {
            KeyCode::Esc => {
                // Cancel — remove the TOTP secret we just set up
                let _ = secrets_manager.remove_totp();
                (None, Action::Noop)
            }
            KeyCode::Char(c) if c.is_ascii_digit() && input.len() < 6 => {
                input.push(c);
                (Some(dlg), Action::Noop)
            }
            KeyCode::Backspace => {
                input.pop();
                (Some(dlg), Action::Noop)
            }
            KeyCode::Enter => {
                if input.len() == 6 {
                    match secrets_manager.verify_totp(input) {
                        Ok(true) => {
                            config.totp_enabled = true;
                            let _ = config.save(None);
                            messages.push(DisplayMessage::success(
                                "✓ 2FA configured successfully.",
                            ));
                            (
                                Some(TotpDialogState {
                                    phase: TotpDialogPhase::Verified,
                                }),
                                Action::Noop,
                            )
                        }
                        Ok(false) => {
                            let saved_uri = uri.clone();
                            (
                                Some(TotpDialogState {
                                    phase: TotpDialogPhase::Failed {
                                        uri: saved_uri,
                                        input: String::new(),
                                    },
                                }),
                                Action::Noop,
                            )
                        }
                        Err(e) => {
                            messages.push(DisplayMessage::error(format!(
                                "TOTP verification error: {}",
                                e,
                            )));
                            let _ = secrets_manager.remove_totp();
                            (None, Action::Noop)
                        }
                    }
                } else {
                    (Some(dlg), Action::Noop)
                }
            }
            _ => (Some(dlg), Action::Noop),
        },
        TotpDialogPhase::AlreadyConfigured => match code {
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                // Keep 2FA
                (None, Action::Noop)
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                // Remove 2FA
                match secrets_manager.remove_totp() {
                    Ok(()) => {
                        config.totp_enabled = false;
                        let _ = config.save(None);
                        messages.push(DisplayMessage::info("2FA has been removed."));
                    }
                    Err(e) => {
                        messages.push(DisplayMessage::error(format!(
                            "Failed to remove 2FA: {}",
                            e,
                        )));
                    }
                }
                (None, Action::Noop)
            }
            _ => (Some(dlg), Action::Noop),
        },
        TotpDialogPhase::Verified => {
            // Any key closes
            (None, Action::Noop)
        }
    }
}


/// Handle key events for the TOTP dialog (gateway-backed).
/// Returns Actions that trigger gateway sends instead of calling SecretsManager directly.
pub fn handle_totp_dialog_key_gateway(
    dlg: TotpDialogState,
    code: crossterm::event::KeyCode,
    messages: &mut Vec<DisplayMessage>,
) -> (Option<TotpDialogState>, Action) {
    use crossterm::event::KeyCode;

    let mut dlg = dlg;

    match dlg.phase {
        TotpDialogPhase::ShowUri {
            uri: _,
            ref mut input,
        }
        | TotpDialogPhase::Failed {
            uri: _,
            ref mut input,
        } => match code {
            KeyCode::Esc => {
                // Cancel — remove the TOTP secret via gateway
                let frame = serde_json::json!({"type": "secrets_remove_totp"});
                (None, Action::SendToGateway(frame.to_string()))
            }
            KeyCode::Char(c) if c.is_ascii_digit() && input.len() < 6 => {
                input.push(c);
                (Some(dlg), Action::Noop)
            }
            KeyCode::Backspace => {
                input.pop();
                (Some(dlg), Action::Noop)
            }
            KeyCode::Enter => {
                if input.len() == 6 {
                    // Send verification to gateway
                    let frame = serde_json::json!({
                        "type": "secrets_verify_totp",
                        "code": input,
                    });
                    // Keep the dialog open — result will be handled by app.rs
                    (Some(dlg), Action::SendToGateway(frame.to_string()))
                } else {
                    (Some(dlg), Action::Noop)
                }
            }
            _ => (Some(dlg), Action::Noop),
        },
        TotpDialogPhase::AlreadyConfigured => match code {
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                (None, Action::Noop)
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                // Remove 2FA via gateway
                let frame = serde_json::json!({"type": "secrets_remove_totp"});
                messages.push(DisplayMessage::info("Removing 2FA…"));
                (None, Action::SendToGateway(frame.to_string()))
            }
            _ => (Some(dlg), Action::Noop),
        },
        TotpDialogPhase::Verified => {
            // Any key closes
            (None, Action::Noop)
        }
    }
}

/// Draw a centered TOTP setup dialog overlay.
pub fn draw_totp_dialog(frame: &mut ratatui::Frame<'_>, area: Rect, dlg: &TotpDialogState) {
    let dialog_w = 56.min(area.width.saturating_sub(4));
    let dialog_h = 12u16.min(area.height.saturating_sub(4)).max(8);
    let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
    let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

    frame.render_widget(Clear, dialog_area);

    let (title, lines, hint): (&str, Vec<Line>, &str) = match &dlg.phase {
        TotpDialogPhase::ShowUri { uri, input } => {
            let masked: String = format!("{}{}", "*".repeat(input.len()), "_".repeat(6 - input.len()));
            (
                " Set up 2FA ",
                vec![
                    Line::from(Span::styled(
                        "Add this URI to your authenticator app:",
                        Style::default().fg(tp::TEXT),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        uri.as_str(),
                        Style::default().fg(tp::ACCENT_BRIGHT),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        "Enter the 6-digit code to verify:",
                        Style::default().fg(tp::TEXT),
                    )),
                    Line::from(Span::styled(
                        format!("  Code: [{}]", masked),
                        Style::default().fg(tp::WARN).add_modifier(Modifier::BOLD),
                    )),
                ],
                " Enter code · Esc cancel ",
            )
        }
        TotpDialogPhase::Failed { input, .. } => {
            let masked: String = format!("{}{}", "*".repeat(input.len()), "_".repeat(6 - input.len()));
            (
                " 2FA Verification ",
                vec![
                    Line::from(Span::styled(
                        "✗ Code invalid — please try again.",
                        Style::default().fg(tp::ERROR).add_modifier(Modifier::BOLD),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        format!("  Code: [{}]", masked),
                        Style::default().fg(tp::WARN).add_modifier(Modifier::BOLD),
                    )),
                ],
                " Enter code · Esc cancel ",
            )
        }
        TotpDialogPhase::AlreadyConfigured => (
            " 2FA Active ",
            vec![
                Line::from(Span::styled(
                    "Two-factor authentication is already configured.",
                    Style::default().fg(tp::SUCCESS),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Remove 2FA? (y/n)",
                    Style::default().fg(tp::WARN),
                )),
            ],
            " y remove · n/Esc keep ",
        ),
        TotpDialogPhase::Verified => (
            " 2FA Configured ",
            vec![
                Line::from(Span::styled(
                    "✓ Two-factor authentication is now active.",
                    Style::default()
                        .fg(tp::SUCCESS)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Credentials with AUTH policy will require TOTP.",
                    Style::default().fg(tp::TEXT_DIM),
                )),
            ],
            " Press any key to close ",
        ),
    };

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

    let text = ratatui::text::Text::from(lines);
    let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}
