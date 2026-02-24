// ── Secrets dialog — interactive vault management overlay ────────────────────

use iocraft::prelude::*;
use crate::theme;

#[derive(Debug, Clone, Default)]
pub struct SecretInfo {
    pub name: String,
    pub label: String,
    pub kind: String,
    pub policy: String,
    pub disabled: bool,
}

#[derive(Default, Props)]
pub struct SecretsDialogProps {
    pub secrets: Vec<SecretInfo>,
    pub agent_access: bool,
    pub has_totp: bool,
    pub selected: Option<usize>,
    pub scroll_offset: usize,
    /// 0 = normal, 1 = entering name, 2 = entering value
    pub add_step: u8,
    pub add_name: String,
    pub add_value: String,
}

#[component]
pub fn SecretsDialog(props: &SecretsDialogProps) -> impl Into<AnyElement<'static>> {
    let access_label = if props.agent_access { "Enabled" } else { "Disabled" };
    let access_color = if props.agent_access { theme::SUCCESS } else { theme::WARN };
    let totp_label = if props.has_totp { "On" } else { "Off" };
    let totp_color = if props.has_totp { theme::SUCCESS } else { theme::MUTED };
    let count = props.secrets.len();
    let sel = props.selected.unwrap_or(0);

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
                border_color: theme::ACCENT_BRIGHT,
                background_color: theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
                overflow: Overflow::Hidden,
            ) {
                // Title
                Text(
                    content: "🔐 Secrets Vault",
                    color: theme::ACCENT_BRIGHT,
                    weight: Weight::Bold,
                )

                View(height: 1)

                // Summary line
                View(flex_direction: FlexDirection::Row) {
                    Text(content: "Agent Access: ", color: theme::TEXT_DIM)
                    Text(content: access_label, color: access_color)
                    Text(content: "  │  ", color: theme::MUTED)
                    Text(content: format!("{} credential{}", count, if count == 1 { "" } else { "s" }), color: theme::TEXT_DIM)
                    Text(content: "  │  2FA: ", color: theme::TEXT_DIM)
                    Text(content: totp_label, color: totp_color)
                }

                View(height: 1)

                // Credential list
                #(if props.secrets.is_empty() {
                    element! {
                        Text(content: "  No credentials stored.  Press 'a' to add one.", color: theme::MUTED)
                    }.into_any()
                } else {
                    element! {
                        View(
                            flex_direction: FlexDirection::Column,
                            width: 100pct,
                            overflow: Overflow::Hidden,
                        ) {
                            #(props.secrets.iter().enumerate().skip(props.scroll_offset).take(20).map(|(i, s)| {
                                let is_selected = i == sel;
                                let bg = if is_selected { Some(theme::ACCENT_BRIGHT) } else { None };
                                let pointer = if is_selected { "▸ " } else { "  " };
                                let fg = if is_selected {
                                    theme::BG_MAIN
                                } else if s.disabled {
                                    theme::MUTED
                                } else {
                                    theme::TEXT
                                };
                                let status = if s.disabled { "OFF".to_string() } else { s.policy.clone() };
                                let suffix = if !s.name.is_empty() && s.name != s.label {
                                    format!(" — {}", s.name)
                                } else {
                                    String::new()
                                };
                                let line = format!("{}{:10}  {:5}  {}{}", pointer, s.kind, status, s.label, suffix);
                                element! {
                                    View(
                                        key: i as u64,
                                        width: 100pct,
                                        background_color: bg.unwrap_or(Color::Reset),
                                    ) {
                                        Text(content: line, color: fg, wrap: TextWrap::NoWrap)
                                    }
                                }
                            }))
                        }
                    }.into_any()
                })

                View(height: 1)

                // Add-secret inline input
                #(if props.add_step > 0 {
                    let (label, input_text) = if props.add_step == 1 {
                        ("Name: ", props.add_name.as_str())
                    } else {
                        ("Value: ", props.add_value.as_str())
                    };
                    let cursor_display = format!("{}{}█", label, input_text);
                    element! {
                        View(
                            flex_direction: FlexDirection::Column,
                            width: 100pct,
                        ) {
                            Text(content: "Add Secret", color: theme::ACCENT_BRIGHT, weight: Weight::Bold)
                            View(
                                width: 100pct,
                                border_style: BorderStyle::Round,
                                border_color: theme::ACCENT_BRIGHT,
                                padding_left: 1,
                                padding_right: 1,
                            ) {
                                Text(content: cursor_display, color: theme::TEXT, wrap: TextWrap::NoWrap)
                            }
                            Text(
                                content: if props.add_step == 1 {
                                    "Enter name, then press Enter for value  │  Esc cancel"
                                } else {
                                    "Enter value, then press Enter to save   │  Esc cancel"
                                },
                                color: theme::MUTED,
                            )
                        }
                    }.into_any()
                } else {
                    element! { View() }.into_any()
                })

                // Legend (hide when in add mode)
                #(if props.add_step == 0 {
                    element! {
                        View(flex_direction: FlexDirection::Row) {
                            View(background_color: theme::SUCCESS) {
                                Text(content: " OPEN ", color: theme::BG_MAIN)
                            }
                            Text(content: " anytime  ", color: theme::TEXT_DIM)
                            View(background_color: theme::WARN) {
                                Text(content: " ASK ", color: theme::BG_MAIN)
                            }
                            Text(content: " per-use  ", color: theme::TEXT_DIM)
                            View(background_color: theme::ERROR) {
                                Text(content: " AUTH ", color: theme::BG_MAIN)
                            }
                            Text(content: " re-auth  ", color: theme::TEXT_DIM)
                            View(background_color: theme::INFO) {
                                Text(content: " SKILL ", color: theme::BG_MAIN)
                            }
                            Text(content: " gated", color: theme::TEXT_DIM)
                        }
                    }.into_any()
                } else {
                    element! { View() }.into_any()
                })

                View(height: 1)

                // Hint
                #(if props.add_step == 0 {
                    element! {
                        View(flex_direction: FlexDirection::Row) {
                            Text(content: "↑↓ ", color: theme::ACCENT_BRIGHT)
                            Text(content: "navigate  ", color: theme::MUTED)
                            Text(content: "Enter ", color: theme::ACCENT_BRIGHT)
                            Text(content: "cycle policy  ", color: theme::MUTED)
                            Text(content: "a ", color: theme::ACCENT_BRIGHT)
                            Text(content: "add  ", color: theme::MUTED)
                            Text(content: "d ", color: theme::ACCENT_BRIGHT)
                            Text(content: "delete  ", color: theme::MUTED)
                            Text(content: "Esc ", color: theme::ACCENT_BRIGHT)
                            Text(content: "close", color: theme::MUTED)
                        }
                    }.into_any()
                } else {
                    element! { View() }.into_any()
                })
            }
        }
    }
}
