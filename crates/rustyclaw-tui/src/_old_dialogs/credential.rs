//! Credential management dialog.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

use crate::action::Action;
use crate::panes::DisplayMessage;
use rustyclaw_core::secrets::{AccessPolicy, SecretsManager};
use crate::tui_palette as tp;

use super::secret_viewer::copy_to_clipboard;
use super::{PolicyPickerOption, PolicyPickerPhase, PolicyPickerState, SecretViewerState};

/// Which option is highlighted in the credential-management dialog.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CredDialogOption {
    ViewSecret,
    CopySecret,
    ChangePolicy,
    ToggleDisable,
    Delete,
    SetupTotp,
    Cancel,
}

/// State for the credential-management dialog overlay.
pub struct CredentialDialogState {
    /// Vault key name of the credential
    pub name: String,
    /// Whether the credential is currently disabled
    pub disabled: bool,
    /// Whether 2FA is currently configured for the vault
    pub has_totp: bool,
    /// Current access policy of the credential
    pub current_policy: AccessPolicy,
    /// Currently highlighted menu option
    pub selected: CredDialogOption,
}

/// Handle key events when the credential dialog is open.
/// Returns (updated state or None, optional policy picker, optional secret viewer, action).
pub fn handle_credential_dialog_key(
    dlg: CredentialDialogState,
    code: crossterm::event::KeyCode,
    secrets_manager: &mut SecretsManager,
    messages: &mut Vec<DisplayMessage>,
) -> (
    Option<CredentialDialogState>,
    Option<PolicyPickerState>,
    Option<SecretViewerState>,
    Action,
) {
    use crossterm::event::KeyCode;

    let mut dlg = dlg;

    let options = [
        CredDialogOption::ViewSecret,
        CredDialogOption::CopySecret,
        CredDialogOption::ChangePolicy,
        CredDialogOption::ToggleDisable,
        CredDialogOption::Delete,
        CredDialogOption::SetupTotp,
        CredDialogOption::Cancel,
    ];

    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            // Close without action
            (None, None, None, Action::Noop)
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let cur = options.iter().position(|o| *o == dlg.selected).unwrap_or(0);
            let next = if cur == 0 { options.len() - 1 } else { cur - 1 };
            dlg.selected = options[next];
            (Some(dlg), None, None, Action::Noop)
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let cur = options.iter().position(|o| *o == dlg.selected).unwrap_or(0);
            let next = (cur + 1) % options.len();
            dlg.selected = options[next];
            (Some(dlg), None, None, Action::Noop)
        }
        KeyCode::Enter => {
            match dlg.selected {
                CredDialogOption::ViewSecret => {
                    match secrets_manager.peek_credential_display(&dlg.name) {
                        Ok(fields) => {
                            let viewer = SecretViewerState {
                                name: dlg.name.clone(),
                                fields,
                                revealed: false,
                                selected: 0,
                                scroll_offset: 0,
                                status: None,
                            };
                            (None, None, Some(viewer), Action::Noop)
                        }
                        Err(e) => {
                            messages.push(DisplayMessage::error(format!(
                                "Failed to read secret: {}",
                                e,
                            )));
                            (None, None, None, Action::Noop)
                        }
                    }
                }
                CredDialogOption::CopySecret => {
                    match secrets_manager.peek_credential_display(&dlg.name) {
                        Ok(fields) => {
                            // Copy the first (or only) value to clipboard.
                            let text = if fields.len() == 1 {
                                fields[0].1.clone()
                            } else {
                                // Multi-field: join as "Label: Value" lines.
                                fields
                                    .iter()
                                    .map(|(k, v)| format!("{}: {}", k, v))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            };
                            match copy_to_clipboard(&text) {
                                Ok(()) => {
                                    messages.push(DisplayMessage::success(format!(
                                        "Credential '{}' copied to clipboard.",
                                        dlg.name,
                                    )));
                                }
                                Err(e) => {
                                    messages.push(DisplayMessage::error(format!(
                                        "Failed to copy: {}",
                                        e,
                                    )));
                                }
                            }
                        }
                        Err(e) => {
                            messages.push(DisplayMessage::error(format!(
                                "Failed to read secret: {}",
                                e,
                            )));
                        }
                    }
                    (None, None, None, Action::Update)
                }
                CredDialogOption::ChangePolicy => {
                    // Determine the currently selected policy picker option
                    let selected = match &dlg.current_policy {
                        AccessPolicy::Always => PolicyPickerOption::Open,
                        AccessPolicy::WithApproval => PolicyPickerOption::Ask,
                        AccessPolicy::WithAuth => PolicyPickerOption::Auth,
                        AccessPolicy::SkillOnly(_) => PolicyPickerOption::Skill,
                    };
                    let picker = PolicyPickerState {
                        cred_name: dlg.name.clone(),
                        selected,
                        phase: PolicyPickerPhase::Selecting,
                    };
                    (None, Some(picker), None, Action::Noop)
                }
                CredDialogOption::ToggleDisable => {
                    let new_state = !dlg.disabled;
                    match secrets_manager.set_credential_disabled(&dlg.name, new_state) {
                        Ok(()) => {
                            let verb = if new_state { "disabled" } else { "enabled" };
                            messages.push(DisplayMessage::success(format!(
                                "Credential '{}' {}.",
                                dlg.name, verb,
                            )));
                        }
                        Err(e) => {
                            messages.push(DisplayMessage::error(format!(
                                "Failed to update credential: {}",
                                e,
                            )));
                        }
                    }
                    (None, None, None, Action::Update)
                }
                CredDialogOption::Delete => {
                    // For legacy bare keys, also delete the raw key
                    let meta_key = format!("cred:{}", dlg.name);
                    let is_legacy = secrets_manager
                        .get_secret(&meta_key, true)
                        .ok()
                        .flatten()
                        .is_none();

                    if is_legacy {
                        let _ = secrets_manager.delete_secret(&dlg.name);
                    }
                    match secrets_manager.delete_credential(&dlg.name) {
                        Ok(()) => {
                            messages.push(DisplayMessage::success(format!(
                                "Credential '{}' deleted.",
                                dlg.name,
                            )));
                        }
                        Err(e) => {
                            messages.push(DisplayMessage::error(format!(
                                "Failed to delete credential: {}",
                                e,
                            )));
                        }
                    }
                    (None, None, None, Action::Update)
                }
                CredDialogOption::SetupTotp => (None, None, None, Action::ShowTotpSetup),
                CredDialogOption::Cancel => {
                    // Close
                    (None, None, None, Action::Update)
                }
            }
        }
        _ => (Some(dlg), None, None, Action::Noop),
    }
}


/// Handle key events when the credential dialog is open (gateway-backed).
/// Instead of calling SecretsManager directly, returns Actions that the app
/// dispatches as gateway sends.
pub fn handle_credential_dialog_key_gateway(
    dlg: CredentialDialogState,
    code: crossterm::event::KeyCode,
    _messages: &mut Vec<DisplayMessage>,
) -> (
    Option<CredentialDialogState>,
    Option<PolicyPickerState>,
    Action,
) {
    use crossterm::event::KeyCode;

    let mut dlg = dlg;

    let options = [
        CredDialogOption::ViewSecret,
        CredDialogOption::CopySecret,
        CredDialogOption::ChangePolicy,
        CredDialogOption::ToggleDisable,
        CredDialogOption::Delete,
        CredDialogOption::SetupTotp,
        CredDialogOption::Cancel,
    ];

    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            (None, None, Action::Noop)
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let cur = options.iter().position(|o| *o == dlg.selected).unwrap_or(0);
            let next = if cur == 0 { options.len() - 1 } else { cur - 1 };
            dlg.selected = options[next];
            (Some(dlg), None, Action::Noop)
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let cur = options.iter().position(|o| *o == dlg.selected).unwrap_or(0);
            let next = (cur + 1) % options.len();
            dlg.selected = options[next];
            (Some(dlg), None, Action::Noop)
        }
        KeyCode::Enter => {
            match dlg.selected {
                CredDialogOption::ViewSecret => {
                    // Request peek from gateway
                    let frame = serde_json::json!({
                        "type": "secrets_peek",
                        "name": dlg.name,
                    });
                    (None, None, Action::SendToGateway(frame.to_string()))
                }
                CredDialogOption::CopySecret => {
                    // Request peek from gateway, then copy
                    // For now, just send a peek request — the app.rs handler
                    // will open the secret viewer which has a copy function.
                    let frame = serde_json::json!({
                        "type": "secrets_peek",
                        "name": dlg.name,
                    });
                    (None, None, Action::SendToGateway(frame.to_string()))
                }
                CredDialogOption::ChangePolicy => {
                    let selected = match &dlg.current_policy {
                        AccessPolicy::Always => PolicyPickerOption::Open,
                        AccessPolicy::WithApproval => PolicyPickerOption::Ask,
                        AccessPolicy::WithAuth => PolicyPickerOption::Auth,
                        AccessPolicy::SkillOnly(_) => PolicyPickerOption::Skill,
                    };
                    let picker = PolicyPickerState {
                        cred_name: dlg.name.clone(),
                        selected,
                        phase: PolicyPickerPhase::Selecting,
                    };
                    (None, Some(picker), Action::Noop)
                }
                CredDialogOption::ToggleDisable => {
                    let new_state = !dlg.disabled;
                    let frame = serde_json::json!({
                        "type": "secrets_set_disabled",
                        "name": dlg.name,
                        "disabled": new_state,
                    });
                    (None, None, Action::SendToGateway(frame.to_string()))
                }
                CredDialogOption::Delete => {
                    let frame = serde_json::json!({
                        "type": "secrets_delete_credential",
                        "name": dlg.name,
                    });
                    (None, None, Action::SendToGateway(frame.to_string()))
                }
                CredDialogOption::SetupTotp => (None, None, Action::ShowTotpSetup),
                CredDialogOption::Cancel => {
                    (None, None, Action::Update)
                }
            }
        }
        _ => (Some(dlg), None, Action::Noop),
    }
}

/// Draw a centered credential-management dialog overlay.
pub fn draw_credential_dialog(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    dlg: &CredentialDialogState,
) {
    let dialog_w = 50.min(area.width.saturating_sub(4));
    let dialog_h = 14u16.min(area.height.saturating_sub(4)).max(9);
    let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
    let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

    frame.render_widget(Clear, dialog_area);

    let title = format!(" {} ", dlg.name);
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

    let toggle_label = if dlg.disabled {
        "  Enable credential"
    } else {
        "  Disable credential"
    };

    let totp_label = if dlg.has_totp {
        "🔒 Manage 2FA (TOTP)"
    } else {
        "🔒 Set up 2FA (TOTP)"
    };

    let policy_label = format!("🛡 Change policy [{}]", dlg.current_policy.badge());

    let menu_items: Vec<(String, CredDialogOption)> = vec![
        ("👁 View secret".to_string(), CredDialogOption::ViewSecret),
        (
            "📋 Copy to clipboard".to_string(),
            CredDialogOption::CopySecret,
        ),
        (policy_label, CredDialogOption::ChangePolicy),
        (toggle_label.to_string(), CredDialogOption::ToggleDisable),
        (
            "  Delete credential".to_string(),
            CredDialogOption::Delete,
        ),
        (totp_label.to_string(), CredDialogOption::SetupTotp),
        ("  Cancel".to_string(), CredDialogOption::Cancel),
    ];

    let items: Vec<ListItem> = menu_items
        .iter()
        .map(|(label, opt)| {
            let is_selected = *opt == dlg.selected;
            let (marker, style) = if is_selected {
                (
                    "❯ ",
                    Style::default()
                        .fg(tp::ACCENT_BRIGHT)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                ("  ", Style::default().fg(tp::TEXT))
            };

            // Colour the delete option red when highlighted
            let final_style = if *opt == CredDialogOption::Delete && is_selected {
                style.fg(tp::ERROR)
            } else {
                style
            };

            ListItem::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(tp::ACCENT)),
                Span::styled(label.as_str(), final_style),
            ]))
        })
        .collect();

    let list = List::new(items).style(Style::default().fg(tp::TEXT));
    frame.render_widget(list, inner);
}
