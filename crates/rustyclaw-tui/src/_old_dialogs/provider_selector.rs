//! Provider selector dialog.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

use crate::action::Action;
use crate::panes::DisplayMessage;
use rustyclaw_core::providers;
use crate::tui_palette as tp;

/// State for the provider-selector dialog overlay.
pub struct ProviderSelectorState {
    /// Provider entries: (id, display)
    pub providers: Vec<(String, String)>,
    /// Currently highlighted index
    pub selected: usize,
    /// Scroll offset
    pub scroll_offset: usize,
}

/// Maximum visible rows in the dialog body.
const MAX_VISIBLE: usize = 14;

/// Open the provider-selector dialog populated from the shared
/// provider registry.
pub fn open_provider_selector() -> ProviderSelectorState {
    let providers: Vec<(String, String)> = providers::PROVIDERS
        .iter()
        .map(|p| (p.id.to_string(), p.display.to_string()))
        .collect();
    ProviderSelectorState {
        providers,
        selected: 0,
        scroll_offset: 0,
    }
}

/// Handle key events when the provider selector dialog is open.
/// Returns (updated state or None if closed, action to dispatch).
pub fn handle_provider_selector_key(
    mut sel: ProviderSelectorState,
    code: crossterm::event::KeyCode,
    messages: &mut Vec<DisplayMessage>,
) -> (Option<ProviderSelectorState>, Action) {
    use crossterm::event::KeyCode;

    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            messages.push(DisplayMessage::info("Provider selection cancelled."));
            (None, Action::Noop)
        }
        KeyCode::Enter => {
            if let Some((id, display)) = sel.providers.get(sel.selected).cloned() {
                messages.push(DisplayMessage::info(format!(
                    "Switching provider to {}\u{2026}",
                    display,
                )));
                (None, Action::SetProvider(id))
            } else {
                (None, Action::Noop)
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if sel.selected > 0 {
                sel.selected -= 1;
                if sel.selected < sel.scroll_offset {
                    sel.scroll_offset = sel.selected;
                }
            }
            (Some(sel), Action::Noop)
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if sel.selected + 1 < sel.providers.len() {
                sel.selected += 1;
                if sel.selected >= sel.scroll_offset + MAX_VISIBLE {
                    sel.scroll_offset = sel.selected - MAX_VISIBLE + 1;
                }
            }
            (Some(sel), Action::Noop)
        }
        KeyCode::Home => {
            sel.selected = 0;
            sel.scroll_offset = 0;
            (Some(sel), Action::Noop)
        }
        KeyCode::End => {
            sel.selected = sel.providers.len().saturating_sub(1);
            sel.scroll_offset = sel.providers.len().saturating_sub(MAX_VISIBLE);
            (Some(sel), Action::Noop)
        }
        _ => (Some(sel), Action::Noop),
    }
}

/// Draw a centered provider-selector dialog overlay.
pub fn draw_provider_selector_dialog(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    sel: &ProviderSelectorState,
) {
    let dialog_w = 50.min(area.width.saturating_sub(4));
    let visible_count = sel.providers.len().min(MAX_VISIBLE);
    let dialog_h = ((visible_count as u16) + 4)
        .min(area.height.saturating_sub(4))
        .max(6);
    let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
    let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

    frame.render_widget(Clear, dialog_area);

    let title = " Select a provider ";
    let hint = if sel.providers.len() > MAX_VISIBLE {
        format!(
            " {}/{} · ↑↓ navigate · Enter select · Esc cancel ",
            sel.selected + 1,
            sel.providers.len(),
        )
    } else {
        " ↑↓ navigate · Enter select · Esc cancel ".to_string()
    };

    let block = Block::default()
        .title(Span::styled(title, tp::title_focused()))
        .title_bottom(
            Line::from(Span::styled(&hint, Style::default().fg(tp::MUTED))).right_aligned(),
        )
        .borders(Borders::ALL)
        .border_style(tp::focused_border())
        .border_type(ratatui::widgets::BorderType::Rounded);

    let inner = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

    let end = (sel.scroll_offset + MAX_VISIBLE).min(sel.providers.len());
    let visible = &sel.providers[sel.scroll_offset..end];

    let items: Vec<ListItem> = visible
        .iter()
        .enumerate()
        .map(|(i, (_id, display))| {
            let abs_idx = sel.scroll_offset + i;
            let is_selected = abs_idx == sel.selected;
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
            ListItem::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(tp::ACCENT)),
                Span::styled(display.as_str(), style),
            ]))
        })
        .collect();

    let list = List::new(items).style(Style::default().fg(tp::TEXT));

    frame.render_widget(list, inner);
}
