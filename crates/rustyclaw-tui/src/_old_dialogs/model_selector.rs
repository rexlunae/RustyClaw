//! Model selector dialog.

use tokio::sync::mpsc;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

use crate::action::Action;
use rustyclaw_core::config::ModelProvider;
use crate::panes::DisplayMessage;
use rustyclaw_core::providers;
use rustyclaw_core::secrets::SecretsManager;
use crate::tui_palette as tp;

use super::SPINNER_FRAMES;

/// Spinner state shown while fetching models from a provider API.
pub struct FetchModelsLoading {
    /// Display name of the provider
    pub display: String,
    /// Tick counter for the spinner animation
    pub tick: usize,
}

/// State for the model-selector dialog overlay.
pub struct ModelSelectorState {
    /// Provider this selection is for
    pub provider: String,
    /// Display name
    pub display: String,
    /// Available model names
    pub models: Vec<String>,
    /// Currently highlighted index
    pub selected: usize,
    /// Scroll offset when the list is longer than the dialog
    pub scroll_offset: usize,
}

/// Maximum visible rows in the dialog body.
const MAX_VISIBLE: usize = 14;

/// Spawn a background task to fetch models and send the result back
/// via the action channel. Shows an inline loading line in the meantime.
pub fn spawn_fetch_models(
    provider: &str,
    secrets_manager: &mut SecretsManager,
    model_config: Option<&ModelProvider>,
    messages: &mut Vec<DisplayMessage>,
    loading_line: &mut Option<String>,
    fetch_loading: &mut Option<FetchModelsLoading>,
    action_tx: mpsc::UnboundedSender<Action>,
) {
    let display = providers::display_name_for_provider(provider).to_string();
    messages.push(DisplayMessage::info(format!(
        "Fetching available models for {}…",
        display,
    )));

    // Show the inline loading line under the chat log
    let spinner = SPINNER_FRAMES[0];
    *loading_line = Some(format!("  {} Fetching models from {}…", spinner, display));
    *fetch_loading = Some(FetchModelsLoading {
        display: display.clone(),
        tick: 0,
    });

    // Gather what we need for the background task
    let api_key = providers::secret_key_for_provider(provider).and_then(|sk| {
        secrets_manager
            .get_secret(sk, true)
            .ok()
            .flatten()
    });

    let base_url = model_config.and_then(|m| m.base_url.clone());

    let provider_clone = provider.to_string();

    tokio::spawn(async move {
        match providers::fetch_models(&provider_clone, api_key.as_deref(), base_url.as_deref())
            .await
        {
            Ok(models) => {
                let _ = action_tx.send(Action::ShowModelSelector {
                    provider: provider_clone,
                    models,
                });
            }
            Err(err) => {
                let _ = action_tx.send(Action::FetchModelsFailed(err));
            }
        }
    });
}

/// Open the model selector dialog with the given list.
pub fn open_model_selector(provider: &str, models: Vec<String>) -> ModelSelectorState {
    let display = providers::display_name_for_provider(provider).to_string();
    ModelSelectorState {
        provider: provider.to_string(),
        display,
        models,
        selected: 0,
        scroll_offset: 0,
    }
}

/// Handle key events when the model selector dialog is open.
/// Returns (updated state or None if closed, action to dispatch).
pub fn handle_model_selector_key(
    mut sel: ModelSelectorState,
    code: crossterm::event::KeyCode,
    model_config: &mut Option<ModelProvider>,
    messages: &mut Vec<DisplayMessage>,
    save_config: impl FnOnce() -> Result<(), String>,
) -> (Option<ModelSelectorState>, Action) {
    use crossterm::event::KeyCode;

    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            messages.push(DisplayMessage::info("Model selection cancelled."));
            (None, Action::Noop)
        }
        KeyCode::Enter => {
            if let Some(model_name) = sel.models.get(sel.selected).cloned() {
                // Save the selected model
                let cfg = model_config.get_or_insert_with(|| ModelProvider {
                    provider: sel.provider.clone(),
                    model: None,
                    base_url: None,
                });
                cfg.model = Some(model_name.clone());
                if let Err(e) = save_config() {
                    messages.push(DisplayMessage::error(format!("Failed to save config: {}", e)));
                } else {
                    messages.push(DisplayMessage::success(format!(
                        "✓ Model set to {}.",
                        model_name,
                    )));
                }
            }
            // Restart the gateway so it picks up the new model.
            (None, Action::RestartGateway)
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
            if sel.selected + 1 < sel.models.len() {
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
            sel.selected = sel.models.len().saturating_sub(1);
            sel.scroll_offset = sel.models.len().saturating_sub(MAX_VISIBLE);
            (Some(sel), Action::Noop)
        }
        _ => (Some(sel), Action::Noop),
    }
}

/// Draw a centered model-selector dialog overlay.
pub fn draw_model_selector_dialog(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    sel: &ModelSelectorState,
) {
    let dialog_w = 60.min(area.width.saturating_sub(4));
    let visible_count = sel.models.len().min(MAX_VISIBLE);
    // +4 for border (2) + title line + hint line
    let dialog_h = ((visible_count as u16) + 4)
        .min(area.height.saturating_sub(4))
        .max(6);
    let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
    let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

    // Clear the background behind the dialog
    frame.render_widget(Clear, dialog_area);

    let title = format!(" Select a {} model ", sel.display);
    let hint = if sel.models.len() > MAX_VISIBLE {
        format!(
            " {}/{} · ↑↓ navigate · Enter select · Esc cancel ",
            sel.selected + 1,
            sel.models.len(),
        )
    } else {
        " ↑↓ navigate · Enter select · Esc cancel ".to_string()
    };

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

    let end = (sel.scroll_offset + MAX_VISIBLE).min(sel.models.len());
    let visible_models = &sel.models[sel.scroll_offset..end];

    let items: Vec<ListItem> = visible_models
        .iter()
        .enumerate()
        .map(|(i, model)| {
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
                Span::styled(model.as_str(), style),
            ]))
        })
        .collect();

    let list = List::new(items).style(Style::default().fg(tp::TEXT));

    frame.render_widget(list, inner);
}
