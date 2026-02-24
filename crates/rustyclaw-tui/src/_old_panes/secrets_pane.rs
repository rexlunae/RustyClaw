use anyhow::Result;
use ratatui::{
    layout::{Constraint, Rect},
    prelude::*,
    widgets::{Block, BorderType, Borders, List, ListItem, Scrollbar, ScrollbarOrientation, ScrollbarState},
};

use crate::action::Action;
use crate::panes::{Pane, PaneState};
use rustyclaw_core::secrets::{AccessPolicy, SecretEntry};
use crate::tui_palette as tp;
use crate::tui::Frame;

pub struct SecretsPane {
    focused: bool,
    focused_border_style: Style,
    /// Cached list of (name, metadata) for display — refreshed on
    /// focus and every tick while focused.
    cached_creds: Vec<(String, SecretEntry)>,
    /// Whether the agent has blanket access (cached for draw).
    cached_agent_access: bool,
    /// Whether 2FA (TOTP) is configured for the vault.
    cached_has_totp: bool,
    /// Scroll offset into the credentials list.
    scroll_offset: usize,
}

impl SecretsPane {
    pub fn new(focused: bool, focused_border_style: Style) -> Self {
        Self {
            focused,
            focused_border_style,
            cached_creds: Vec::new(),
            cached_agent_access: false,
            cached_has_totp: false,
            scroll_offset: 0,
        }
    }

    fn border_style(&self) -> Style {
        if self.focused {
            self.focused_border_style
        } else {
            tp::unfocused_border()
        }
    }

    fn border_type(&self) -> BorderType {
        if self.focused {
            BorderType::Thick
        } else {
            BorderType::Plain
        }
    }

    /// Refresh the cached credentials snapshot from the vault.
    fn refresh_cache(&mut self, state: &mut PaneState<'_>) {
        self.cached_creds = state.secrets_manager.list_all_entries();
        self.cached_agent_access = state.secrets_manager.has_agent_access();
        self.cached_has_totp = state.secrets_manager.has_totp();
        // Clamp scroll
        let max = self.cached_creds.len().saturating_sub(1);
        if self.scroll_offset > max {
            self.scroll_offset = max;
        }
    }

    /// Build a styled policy badge span.
    fn policy_badge(policy: &AccessPolicy) -> Span<'static> {
        let (label, color) = match policy {
            AccessPolicy::Always => (" OPEN ", tp::SUCCESS),
            AccessPolicy::WithApproval => (" ASK ", tp::WARN),
            AccessPolicy::WithAuth => (" AUTH ", tp::ERROR),
            AccessPolicy::SkillOnly(skills) if skills.is_empty() => (" LOCK ", tp::MUTED),
            AccessPolicy::SkillOnly(_) => (" SKILL ", tp::INFO),
        };
        Span::styled(label, Style::default().fg(Color::Rgb(0x1E, 0x1C, 0x1A)).bg(color))
    }
}

impl Pane for SecretsPane {
    fn height_constraint(&self) -> Constraint {
        match self.focused {
            true => Constraint::Fill(3),
            false => Constraint::Fill(1),
        }
    }

    fn update(&mut self, action: Action, state: &mut PaneState<'_>) -> Result<Option<Action>> {
        match action {
            Action::Focus => {
                self.focused = true;
                self.refresh_cache(state);
                return Ok(Some(Action::TimedStatusLine(
                    "[secrets pane focused]".into(),
                    3,
                )));
            }
            Action::UnFocus => {
                self.focused = false;
            }
            Action::Tick => {
                // Refresh credential snapshot each tick so the list
                // stays current even before the pane is focused.
                self.refresh_cache(state);
            }
            Action::Down => {
                if self.focused && !self.cached_creds.is_empty() {
                    let max = self.cached_creds.len().saturating_sub(1);
                    self.scroll_offset = (self.scroll_offset + 1).min(max);
                }
            }
            Action::Up => {
                if self.focused {
                    self.scroll_offset = self.scroll_offset.saturating_sub(1);
                }
            }
            Action::Submit => {
                if self.focused {
                    if let Some((name, entry)) = self.cached_creds.get(self.scroll_offset) {
                        return Ok(Some(Action::ShowCredentialDialog {
                            name: name.clone(),
                            disabled: entry.disabled,
                            policy: entry.policy.badge().to_string(),
                        }));
                    }
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn init(&mut self, _state: &PaneState<'_>) -> Result<()> {
        // Initial cache will be populated on first Tick.
        Ok(())
    }

    fn draw(&mut self, frame: &mut Frame<'_>, area: Rect, _state: &PaneState<'_>) -> Result<()> {
        // ── Header: agent-access status line ────────────────────────
        let (access_label, access_style) = if self.cached_agent_access {
            ("Enabled", Style::default().fg(tp::SUCCESS))
        } else {
            ("Disabled", Style::default().fg(tp::WARN))
        };

        let mut items: Vec<ListItem> = Vec::new();

        items.push(ListItem::new(Line::from(vec![
            Span::styled("Agent Access: ", Style::default().fg(tp::TEXT_DIM)),
            Span::styled(access_label, access_style),
            Span::styled(
                format!("  │  {} credential{}",
                    self.cached_creds.len(),
                    if self.cached_creds.len() == 1 { "" } else { "s" },
                ),
                Style::default().fg(tp::TEXT_DIM),
            ),
            Span::styled("  │  2FA: ", Style::default().fg(tp::TEXT_DIM)),
            if self.cached_has_totp {
                Span::styled("On", Style::default().fg(tp::SUCCESS))
            } else {
                Span::styled("Off", Style::default().fg(tp::MUTED))
            },
        ])));

        items.push(ListItem::new(""));

        // ── Credential rows ─────────────────────────────────────────
        if self.cached_creds.is_empty() {
            items.push(ListItem::new(Span::styled(
                "  No credentials stored.",
                Style::default().fg(tp::MUTED).add_modifier(Modifier::ITALIC),
            )));
            items.push(ListItem::new(""));
        } else {
            for (i, (name, entry)) in self.cached_creds.iter().enumerate() {
                let highlight = self.focused && i == self.scroll_offset;
                let is_disabled = entry.disabled;

                let row_style = if highlight {
                    tp::selected()
                } else if is_disabled {
                    Style::default().add_modifier(Modifier::DIM | Modifier::CROSSED_OUT)
                } else {
                    Style::default()
                };

                let icon = entry.kind.icon();
                let kind_label = format!(" {:10} ", entry.kind.to_string());
                let badge = if is_disabled {
                    Span::styled(
                        " OFF ",
                        Style::default()
                            .fg(Color::Rgb(0x1E, 0x1C, 0x1A))
                            .bg(tp::MUTED),
                    )
                } else {
                    Self::policy_badge(&entry.policy)
                };

                let desc = entry.description.as_deref().unwrap_or("");
                let detail = if desc.is_empty() {
                    format!(" {}", name)
                } else {
                    format!(" {} — {}", name, desc)
                };

                let label_style = if is_disabled {
                    Style::default().fg(tp::MUTED).add_modifier(Modifier::CROSSED_OUT).patch(row_style)
                } else {
                    Style::default().fg(tp::TEXT).patch(row_style)
                };

                let kind_style = if is_disabled {
                    Style::default().fg(tp::MUTED).patch(row_style)
                } else {
                    Style::default().fg(tp::ACCENT_BRIGHT).patch(row_style)
                };

                items.push(ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", icon), row_style),
                    Span::styled(kind_label, kind_style),
                    badge,
                    Span::styled(" ", row_style),
                    Span::styled(&entry.label, label_style),
                    Span::styled(detail, Style::default().fg(tp::TEXT_DIM).patch(row_style)),
                ])));
            }
            items.push(ListItem::new(""));
        }

        // ── Legend ───────────────────────────────────────────────────
        items.push(ListItem::new(Line::from(vec![
            Span::styled(" OPEN ", Style::default().fg(Color::Rgb(0x1E, 0x1C, 0x1A)).bg(tp::SUCCESS)),
            Span::styled(" anytime  ", Style::default().fg(tp::TEXT_DIM)),
            Span::styled(" ASK ", Style::default().fg(Color::Rgb(0x1E, 0x1C, 0x1A)).bg(tp::WARN)),
            Span::styled(" per-use  ", Style::default().fg(tp::TEXT_DIM)),
            Span::styled(" AUTH ", Style::default().fg(Color::Rgb(0x1E, 0x1C, 0x1A)).bg(tp::ERROR)),
            Span::styled(" re-auth  ", Style::default().fg(tp::TEXT_DIM)),
            Span::styled(" SKILL ", Style::default().fg(Color::Rgb(0x1E, 0x1C, 0x1A)).bg(tp::INFO)),
            Span::styled(" skill-gated", Style::default().fg(tp::TEXT_DIM)),
        ])));

        // ── Render ──────────────────────────────────────────────────
        let title_style = if self.focused {
            tp::title_focused()
        } else {
            tp::title_unfocused()
        };

        let secrets_list = List::new(items).block(
            Block::default()
                .title(Span::styled(" Secrets ", title_style))
                .borders(Borders::ALL)
                .border_style(self.border_style())
                .border_type(self.border_type()),
        );

        frame.render_widget(secrets_list, area);

        // ── Scrollbar (only when focused and list overflows) ────────
        if self.focused && !self.cached_creds.is_empty() {
            let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .thumb_symbol("█");
            let mut sb_state = ScrollbarState::new(self.cached_creds.len())
                .position(self.scroll_offset);
            frame.render_stateful_widget(sb, area, &mut sb_state);
        }

        Ok(())
    }
}
