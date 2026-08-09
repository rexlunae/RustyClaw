// ── Secrets dialog — interactive vault management overlay ────────────────────
//
// Uses the shared view-layer types from `rustyclaw-view` for consistency
// with the desktop client and gateway protocol.

use iocraft::prelude::*;
use rustyclaw_view::SecretsDialogData;

#[derive(Default, Props)]
pub struct SecretsDialogProps {
    pub data: SecretsDialogData,
}

#[component]
pub fn SecretsDialog(props: &SecretsDialogProps) -> impl Into<AnyElement<'static>> {
    let d = &props.data;
    element! {
        View(width: 100pct, height: 100pct) {
            // The viewer replaces the list while a reveal is in progress, so
            // the plaintext gets the whole dialog and one obvious way out.
            #(if d.reveal_totp_prompt {
                render_reveal_totp_prompt(d)
            } else if let Some((name, fields)) = &d.revealed {
                render_revealed(name, fields)
            } else {
                render_list(d)
            })
        }
    }
}

/// The credential list and its add/delete/policy affordances.
fn render_list(d: &SecretsDialogData) -> AnyElement<'static> {
    let access_label = if d.agent_access {
        "Enabled"
    } else {
        "Disabled"
    };
    let access_color = if d.agent_access {
        crate::theme::SUCCESS
    } else {
        crate::theme::WARN
    };
    let totp_label = if d.has_totp { "On" } else { "Off" };
    let totp_color = if d.has_totp {
        crate::theme::SUCCESS
    } else {
        crate::theme::MUTED
    };
    let count = d.secrets.len();
    let sel = d.selected.unwrap_or(0);

    // Compute visible slice
    let end = (d.scroll_offset + 20).min(d.secrets.len());
    let visible: Vec<_> = d.secrets[d.scroll_offset..end]
        .iter()
        .enumerate()
        .map(|(j, s)| {
            let idx = d.scroll_offset + j;
            let is_selected = idx == sel;
            let fg = if is_selected {
                crate::theme::BG_MAIN
            } else if s.disabled {
                crate::theme::MUTED
            } else {
                crate::theme::TEXT
            };
            let bg = if is_selected {
                Some(crate::theme::ACCENT_BRIGHT)
            } else {
                None
            };
            let pointer = if is_selected { "▸ " } else { "  " };
            let status = if s.disabled {
                "OFF".to_string()
            } else {
                s.policy.clone()
            };
            let suffix = if !s.key.is_empty() && s.key != s.label {
                format!(" — {}", s.key)
            } else {
                String::new()
            };
            let line = format!(
                "{}{:10}  {:5}  {}{}",
                pointer, s.kind, status, s.label, suffix
            );
            (idx, is_selected, fg, bg, line)
        })
        .collect();

    // Add-step display
    let add_is_active = d.add_step > 0;
    let (add_label, add_input) = if add_is_active {
        let (label, input_text) = if d.add_step == 1 {
            ("Name: ", d.add_name.as_str())
        } else {
            ("Value: ", d.add_value.as_str())
        };
        (label.to_string(), input_text.to_string())
    } else {
        (String::new(), String::new())
    };

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 70pct,
                max_height: 80pct,
                flex_direction: FlexDirection::Column,
                border_style: BorderStyle::Round,
                border_color: crate::theme::ACCENT_BRIGHT,
                background_color: crate::theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
                overflow: Overflow::Hidden,
            ) {
                // Title
                Text(
                    content: "🔐 Secrets Vault",
                    color: crate::theme::ACCENT_BRIGHT,
                    weight: Weight::Bold,
                )

                View(height: 1)

                // Summary line
                View(flex_direction: FlexDirection::Row) {
                    Text(content: "Agent Access: ", color: crate::theme::TEXT_DIM)
                    Text(content: access_label, color: access_color)
                    Text(content: "  │  ", color: crate::theme::MUTED)
                    Text(content: format!("{} credential{}", count, if count == 1 { "" } else { "s" }), color: crate::theme::TEXT_DIM)
                    Text(content: "  │  2FA: ", color: crate::theme::TEXT_DIM)
                    Text(content: totp_label, color: totp_color)
                }

                View(height: 1)

                // Credential list
                #(if visible.is_empty() {
                    element! {
                        Text(content: "  No credentials stored.  Press 'a' to add one.", color: crate::theme::MUTED)
                    }.into_any()
                } else {
                    element! {
                        View(
                            flex_direction: FlexDirection::Column,
                            width: 100pct,
                            overflow: Overflow::Hidden,
                        ) {
                            #(visible.iter().map(|(idx, _, fg, bg, line)| {
                                element! {
                                    View(
                                        key: *idx as u64,
                                        width: 100pct,
                                        background_color: bg.unwrap_or(Color::Reset),
                                    ) {
                                        Text(content: line, color: *fg, wrap: TextWrap::NoWrap)
                                    }
                                }
                            }))
                        }
                    }.into_any()
                })

                View(height: 1)

                // Add-secret inline input
                #(if add_is_active {
                    let cursor_display = format!("{}{}█", add_label, add_input);
                    element! {
                        View(
                            flex_direction: FlexDirection::Column,
                            width: 100pct,
                        ) {
                            Text(content: "Add Secret", color: crate::theme::ACCENT_BRIGHT, weight: Weight::Bold)
                            View(
                                width: 100pct,
                                border_style: BorderStyle::Round,
                                border_color: crate::theme::ACCENT_BRIGHT,
                                padding_left: 1,
                                padding_right: 1,
                            ) {
                                Text(content: cursor_display, color: crate::theme::TEXT, wrap: TextWrap::NoWrap)
                            }
                            Text(
                                content: if d.add_step == 1 {
                                    "Enter name, then press Enter for value  │  Esc cancel"
                                } else {
                                    "Enter value, then press Enter to save   │  Esc cancel"
                                },
                                color: crate::theme::MUTED,
                            )
                        }
                    }.into_any()
                } else {
                    element! { View() }.into_any()
                })

                // Legend (hide when in add mode)
                #(if d.add_step == 0 {
                    element! {
                        View(flex_direction: FlexDirection::Row) {
                            View(background_color: crate::theme::SUCCESS) {
                                Text(content: " OPEN ", color: crate::theme::BG_MAIN)
                            }
                            Text(content: " anytime  ", color: crate::theme::TEXT_DIM)
                            View(background_color: crate::theme::WARN) {
                                Text(content: " ASK ", color: crate::theme::BG_MAIN)
                            }
                            Text(content: " per-use  ", color: crate::theme::TEXT_DIM)
                            View(background_color: crate::theme::ERROR) {
                                Text(content: " AUTH ", color: crate::theme::BG_MAIN)
                            }
                            Text(content: " re-auth  ", color: crate::theme::TEXT_DIM)
                            View(background_color: crate::theme::INFO) {
                                Text(content: " SKILL ", color: crate::theme::BG_MAIN)
                            }
                            Text(content: " gated", color: crate::theme::TEXT_DIM)
                        }
                    }.into_any()
                } else {
                    element! { View() }.into_any()
                })

                View(height: 1)

                // Hint
                #(if d.add_step == 0 {
                    element! {
                        View(flex_direction: FlexDirection::Row) {
                            Text(content: "↑↓ ", color: crate::theme::ACCENT_BRIGHT)
                            Text(content: "navigate  ", color: crate::theme::MUTED)
                            Text(content: "Enter ", color: crate::theme::ACCENT_BRIGHT)
                            Text(content: "cycle policy  ", color: crate::theme::MUTED)
                            Text(content: "a ", color: crate::theme::ACCENT_BRIGHT)
                            Text(content: "add  ", color: crate::theme::MUTED)
                            Text(content: "d ", color: crate::theme::ACCENT_BRIGHT)
                            Text(content: "delete  ", color: crate::theme::MUTED)
                            Text(content: "v ", color: crate::theme::ACCENT_BRIGHT)
                            Text(content: "view  ", color: crate::theme::MUTED)
                            Text(content: "Esc ", color: crate::theme::ACCENT_BRIGHT)
                            Text(content: "close", color: crate::theme::MUTED)
                        }
                    }.into_any()
                } else {
                    element! { View() }.into_any()
                })
            }
        }
    }
    .into_any()
}

/// Step-up TOTP prompt shown before a secret's values are revealed.
fn render_reveal_totp_prompt(d: &SecretsDialogData) -> AnyElement<'static> {
    let typed = &d.reveal_code;
    let display = format!(
        "{}{}",
        typed,
        "\u{b7}".repeat(6usize.saturating_sub(typed.len()))
    );
    let has_error = !d.reveal_error.is_empty();
    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 52,
                flex_direction: FlexDirection::Column,
                border_style: BorderStyle::Round,
                border_color: crate::theme::ACCENT_BRIGHT,
                background_color: crate::theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
            ) {
                Text(
                    content: "\u{1f511} Confirm to view secret",
                    color: crate::theme::ACCENT_BRIGHT,
                    weight: Weight::Bold,
                )
                View(height: 1)
                Text(
                    content: "Enter your 6-digit TOTP code to reveal this",
                    color: crate::theme::TEXT,
                )
                Text(
                    content: "secret. One code unlocks viewing for 2 minutes.",
                    color: crate::theme::TEXT,
                )
                View(height: 1)
                View(flex_direction: FlexDirection::Row, justify_content: JustifyContent::Center) {
                    Text(
                        content: format!("  {}  ", display),
                        color: crate::theme::ACCENT_BRIGHT,
                        weight: Weight::Bold,
                    )
                }
                #(if has_error {
                    element! {
                        View(margin_top: 1) {
                            Text(content: d.reveal_error.clone(), color: crate::theme::ERROR)
                        }
                    }.into_any()
                } else {
                    element! { View() }.into_any()
                })
                View(height: 1)
                Text(
                    content: "Enter \u{21a9} confirm  \u{b7}  Esc cancel",
                    color: crate::theme::MUTED,
                )
            }
        }
    }
    .into_any()
}

/// The revealed credential's fields. Dismissing drops them from state.
fn render_revealed(name: &str, fields: &[(String, String)]) -> AnyElement<'static> {
    let rows: Vec<(String, String)> = fields
        .iter()
        .map(|(label, value)| (label.clone(), value.clone()))
        .collect();
    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 70pct,
                max_height: 80pct,
                flex_direction: FlexDirection::Column,
                border_style: BorderStyle::Round,
                border_color: crate::theme::WARN,
                background_color: crate::theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
                overflow: Overflow::Hidden,
            ) {
                Text(
                    content: format!("\u{1f513} {}", name),
                    color: crate::theme::WARN,
                    weight: Weight::Bold,
                )
                View(height: 1)
                #(rows.into_iter().map(|(label, value)| element! {
                    View(flex_direction: FlexDirection::Column, margin_bottom: 1) {
                        Text(content: label, color: crate::theme::TEXT_DIM)
                        Text(content: value, color: crate::theme::TEXT)
                    }
                }).collect::<Vec<_>>())
                View(height: 1)
                View(flex_direction: FlexDirection::Row) {
                    Text(content: "Esc ", color: crate::theme::ACCENT_BRIGHT)
                    Text(content: "hide", color: crate::theme::MUTED)
                }
            }
        }
    }
    .into_any()
}
