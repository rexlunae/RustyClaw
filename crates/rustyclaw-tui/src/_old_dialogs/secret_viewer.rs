//! Secret viewer dialog.

use anyhow::{Context, Result};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};
use std::io::Write;
use std::process::{Command, Stdio};

use crate::action::Action;
use crate::tui_palette as tp;

/// State for the secret-viewer dialog overlay.
pub struct SecretViewerState {
    /// Vault key name of the credential
    pub name: String,
    /// Decrypted (label, value) pairs
    pub fields: Vec<(String, String)>,
    /// Whether the values are currently revealed (unmasked)
    pub revealed: bool,
    /// Which field is highlighted (for copying)
    pub selected: usize,
    /// Scroll offset when the list is longer than the dialog
    #[allow(dead_code)]
    pub scroll_offset: usize,
    /// Transient status message (e.g. "Copied!")
    pub status: Option<String>,
}

/// Copy text to the system clipboard using platform-native tools.
pub fn copy_to_clipboard(text: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
        .context("Failed to launch pbcopy")?;

    #[cfg(target_os = "linux")]
    let mut child = {
        // Try xclip first, fall back to xsel
        Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(Stdio::piped())
            .spawn()
            .or_else(|_| {
                Command::new("xsel")
                    .arg("--clipboard")
                    .stdin(Stdio::piped())
                    .spawn()
            })
            .context("Failed to launch xclip or xsel")?
    };

    #[cfg(target_os = "windows")]
    let mut child = Command::new("clip")
        .stdin(Stdio::piped())
        .spawn()
        .context("Failed to launch clip.exe")?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .context("Failed to write to clipboard process")?;
    }
    child.wait().context("Clipboard process failed")?;
    Ok(())
}

/// Handle key events when the secret viewer dialog is open.
/// Returns (updated state or None if closed, action to dispatch).
pub fn handle_secret_viewer_key(
    viewer: SecretViewerState,
    code: crossterm::event::KeyCode,
) -> (Option<SecretViewerState>, Action) {
    use crossterm::event::KeyCode;

    let mut viewer = viewer;

    // Clear transient status on any keypress
    viewer.status = None;

    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            // Close viewer
            (None, Action::Noop)
        }
        KeyCode::Char('r') => {
            // Toggle reveal/mask
            viewer.revealed = !viewer.revealed;
            (Some(viewer), Action::Noop)
        }
        KeyCode::Char('c') => {
            // Copy the selected field value to clipboard
            if let Some((_label, value)) = viewer.fields.get(viewer.selected) {
                match copy_to_clipboard(value) {
                    Ok(()) => {
                        viewer.status = Some("Copied!".to_string());
                    }
                    Err(e) => {
                        viewer.status = Some(format!("Copy failed: {}", e));
                    }
                }
            }
            (Some(viewer), Action::Noop)
        }
        KeyCode::Char('a') => {
            // Copy all fields to clipboard
            let text = viewer
                .fields
                .iter()
                .map(|(k, v)| format!("{}: {}", k, v))
                .collect::<Vec<_>>()
                .join("\n");
            match copy_to_clipboard(&text) {
                Ok(()) => {
                    viewer.status = Some("All fields copied!".to_string());
                }
                Err(e) => {
                    viewer.status = Some(format!("Copy failed: {}", e));
                }
            }
            (Some(viewer), Action::Noop)
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if viewer.selected > 0 {
                viewer.selected -= 1;
            }
            (Some(viewer), Action::Noop)
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if viewer.selected + 1 < viewer.fields.len() {
                viewer.selected += 1;
            }
            (Some(viewer), Action::Noop)
        }
        _ => (Some(viewer), Action::Noop),
    }
}

/// Draw a centered secret-viewer dialog overlay.
pub fn draw_secret_viewer(frame: &mut ratatui::Frame<'_>, area: Rect, viewer: &SecretViewerState) {
    // Size: width up to 70, height = fields + header/status/hint + borders
    let dialog_w = 70u16.min(area.width.saturating_sub(4));
    let content_lines = viewer.fields.len() as u16 + 3; // fields + blank + status + blank
    let dialog_h = (content_lines + 3)
        .min(area.height.saturating_sub(4))
        .max(8);
    let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
    let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

    frame.render_widget(Clear, dialog_area);

    let title = format!(" {} ", viewer.name);
    let reveal_hint = if viewer.revealed { "r:hide" } else { "r:reveal" };
    let hint = format!(
        " ↑↓ select · c copy · a copy all · {} · Esc close ",
        reveal_hint,
    );

    let block = Block::default()
        .title(Span::styled(&title, tp::title_focused()))
        .title_bottom(
            Line::from(Span::styled(&hint, Style::default().fg(tp::MUTED))).right_aligned(),
        )
        .borders(Borders::ALL)
        .border_style(tp::focused_border())
        .border_type(ratatui::widgets::BorderType::Rounded);

    let inner = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

    let max_label_w = viewer
        .fields
        .iter()
        .map(|(k, _)| k.len())
        .max()
        .unwrap_or(0);

    let mut items: Vec<ListItem> = Vec::new();

    for (i, (label, value)) in viewer.fields.iter().enumerate() {
        let is_selected = i == viewer.selected;
        let display_value = if viewer.revealed {
            value.clone()
        } else {
            "•".repeat(value.len().min(32))
        };

        // Truncate to fit dialog width
        let avail = (dialog_w as usize).saturating_sub(max_label_w + 8);
        let truncated = if display_value.len() > avail {
            format!("{}…", &display_value[..avail.saturating_sub(1)])
        } else {
            display_value
        };

        let (marker, label_style, val_style) = if is_selected {
            (
                "❯ ",
                Style::default()
                    .fg(tp::ACCENT_BRIGHT)
                    .add_modifier(Modifier::BOLD),
                Style::default()
                    .fg(tp::TEXT)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            (
                "  ",
                Style::default().fg(tp::TEXT_DIM),
                Style::default().fg(tp::TEXT),
            )
        };

        items.push(ListItem::new(Line::from(vec![
            Span::styled(marker, Style::default().fg(tp::ACCENT)),
            Span::styled(
                format!("{:>width$}: ", label, width = max_label_w),
                label_style,
            ),
            Span::styled(truncated, val_style),
        ])));
    }

    // Status line
    if let Some(ref status) = viewer.status {
        items.push(ListItem::new(""));
        items.push(ListItem::new(Line::from(Span::styled(
            format!("  {}", status),
            Style::default()
                .fg(tp::SUCCESS)
                .add_modifier(Modifier::BOLD),
        ))));
    }

    let list = List::new(items).style(Style::default().fg(tp::TEXT));
    frame.render_widget(list, inner);
}
