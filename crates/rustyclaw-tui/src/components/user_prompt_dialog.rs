// ── User prompt dialog — agent asks user a question ─────────────────────────

use crate::theme;
use iocraft::prelude::*;
use rustyclaw_core::user_prompt_types::{PromptType, UserPrompt};

#[derive(Default, Props)]
pub struct UserPromptDialogProps {
    /// The question title from the agent.
    pub title: String,
    /// Optional longer description.
    pub description: String,
    /// Current user input text (for text input types).
    pub input: String,
    /// Selected option index (for Select/MultiSelect).
    pub selected: usize,
    /// The prompt type (serialized for prop passing).
    pub prompt_type: Option<PromptType>,
}

#[component]
pub fn UserPromptDialog(props: &UserPromptDialogProps) -> impl Into<AnyElement<'static>> {
    let has_desc = !props.description.is_empty();
    let cursor = "▏";

    // Determine what kind of input to show
    let prompt_type = props.prompt_type.clone().unwrap_or(PromptType::TextInput {
        placeholder: None,
        default: None,
    });

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 70,
                flex_direction: FlexDirection::Column,
                border_style: BorderStyle::Round,
                border_color: theme::ACCENT_BRIGHT,
                background_color: theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
            ) {
                // Title
                Text(
                    content: "❓ Agent Question",
                    color: theme::ACCENT_BRIGHT,
                    weight: Weight::Bold,
                )

                View(height: 1)

                // Question text
                Text(
                    content: props.title.clone(),
                    color: theme::TEXT,
                    weight: Weight::Bold,
                    wrap: TextWrap::Wrap,
                )

                // Description (if any)
                #(if has_desc {
                    element! {
                        View(margin_top: 1) {
                            Text(
                                content: props.description.clone(),
                                color: theme::MUTED,
                                wrap: TextWrap::Wrap,
                            )
                        }
                    }.into_any()
                } else {
                    element! { View() }.into_any()
                })

                View(height: 1)

                // Render input based on prompt type
                #(match &prompt_type {
                    PromptType::Select { options, .. } => {
                        element! {
                            View(flex_direction: FlexDirection::Column) {
                                Text(content: "Select an option (↑/↓ to navigate, Enter to select):", color: theme::MUTED)
                                View(height: 1)
                                #(options.iter().enumerate().map(|(i, opt)| {
                                    let is_selected = i == props.selected;
                                    let prefix = if is_selected { "▶ " } else { "  " };
                                    let fg = if is_selected { theme::ACCENT_BRIGHT } else { theme::TEXT };
                                    element! {
                                        View(key: i as u64, flex_direction: FlexDirection::Column) {
                                            Text(
                                                content: format!("{}{}", prefix, opt.label),
                                                color: fg,
                                                weight: if is_selected { Weight::Bold } else { Weight::Normal },
                                            )
                                            #(if let Some(ref desc) = opt.description {
                                                element! {
                                                    Text(
                                                        content: format!("    {}", desc),
                                                        color: theme::MUTED,
                                                    )
                                                }.into_any()
                                            } else {
                                                element! { View() }.into_any()
                                            })
                                        }
                                    }
                                }))
                            }
                        }.into_any()
                    }
                    PromptType::Confirm { default } => {
                        let yes_selected = props.selected == 0;
                        element! {
                            View(flex_direction: FlexDirection::Row, gap: 2u32) {
                                Text(
                                    content: if yes_selected { "▶ Yes" } else { "  Yes" },
                                    color: if yes_selected { theme::SUCCESS } else { theme::TEXT },
                                    weight: if yes_selected { Weight::Bold } else { Weight::Normal },
                                )
                                Text(
                                    content: if !yes_selected { "▶ No" } else { "  No" },
                                    color: if !yes_selected { theme::ERROR } else { theme::TEXT },
                                    weight: if !yes_selected { Weight::Bold } else { Weight::Normal },
                                )
                            }
                        }.into_any()
                    }
                    PromptType::TextInput { placeholder, .. } => {
                        let placeholder_text = placeholder.clone();
                        let display = if props.input.is_empty() {
                            placeholder_text.unwrap_or_default()
                        } else {
                            props.input.clone()
                        };
                        let is_placeholder = props.input.is_empty();
                        element! {
                            View(flex_direction: FlexDirection::Column) {
                                Text(content: "Your answer:", color: theme::MUTED)
                                View(
                                    flex_direction: FlexDirection::Row,
                                    border_style: BorderStyle::Single,
                                    border_color: theme::ACCENT,
                                    padding_left: 1,
                                    padding_right: 1,
                                    min_height: 1u32,
                                ) {
                                    Text(
                                        content: format!("{}{}", display, cursor),
                                        color: if is_placeholder { theme::MUTED } else { theme::TEXT },
                                    )
                                }
                            }
                        }.into_any()
                    }
                    PromptType::Form { .. } => {
                        // Form: show generic text input for now
                        element! {
                            View(flex_direction: FlexDirection::Column) {
                                Text(content: "Your answer:", color: theme::MUTED)
                                View(
                                    flex_direction: FlexDirection::Row,
                                    border_style: BorderStyle::Single,
                                    border_color: theme::ACCENT,
                                    padding_left: 1,
                                    padding_right: 1,
                                    min_height: 1u32,
                                ) {
                                    Text(
                                        content: format!("{}{}", props.input, cursor),
                                        color: theme::TEXT,
                                    )
                                }
                            }
                        }.into_any()
                    }
                    PromptType::MultiSelect { options, .. } => {
                        // For multi-select, we'd need a Vec<bool> for checked state
                        // For now, render as single-select
                        element! {
                            View(flex_direction: FlexDirection::Column) {
                                Text(content: "Select options (Space to toggle, Enter to confirm):", color: theme::MUTED)
                                View(height: 1)
                                #(options.iter().enumerate().map(|(i, opt)| {
                                    let is_selected = i == props.selected;
                                    let prefix = if is_selected { "▶ [ ] " } else { "  [ ] " };
                                    let fg = if is_selected { theme::ACCENT_BRIGHT } else { theme::TEXT };
                                    element! {
                                        View(key: i as u64) {
                                            Text(
                                                content: format!("{}{}", prefix, opt.label),
                                                color: fg,
                                            )
                                        }
                                    }
                                }))
                            }
                        }.into_any()
                    }
                })

                View(height: 1)

                // Hint based on prompt type
                #(match &prompt_type {
                    PromptType::Select { .. } => {
                        element! {
                            Text(content: "↑↓ navigate  ·  Enter ↩ select  ·  Esc dismiss", color: theme::MUTED)
                        }.into_any()
                    }
                    PromptType::Confirm { .. } => {
                        element! {
                            Text(content: "←→ or Y/N  ·  Enter ↩ confirm  ·  Esc dismiss", color: theme::MUTED)
                        }.into_any()
                    }
                    _ => {
                        element! {
                            Text(content: "Enter ↩ submit  ·  Esc dismiss", color: theme::MUTED)
                        }.into_any()
                    }
                })
            }
        }
    }
}
