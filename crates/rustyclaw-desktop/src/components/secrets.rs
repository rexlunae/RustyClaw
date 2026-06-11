//! Secrets management dialog — view and manage vault credentials.
//!
//! Displays all secrets stored in the gateway vault and emits commands
//! for refresh, store, delete, and set-policy operations.  The parent
//! component manages all mutable state.  The credential list renders
//! as a Bulma `Table`; policy badges and the legend use the shared
//! policy tones from `rustyclaw-view`.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{
    BulmaColor, BulmaSize, Button, Buttons, Delete, Level, LevelItem, LevelLeft, Table, Tag,
};
use rustyclaw_view::SecretsDialogData;
use rustyclaw_view::dialogs::{POLICY_LEGEND, next_policy};

use super::{RcModal, tone_color};

/// Commands emitted by the secrets dialog back to the parent.
#[derive(Clone, Debug)]
pub enum SecretsCommand {
    /// Request secrets list refresh from gateway.
    Refresh,
    /// Store a new secret. Reserved for the add-secret flow, which is not yet
    /// wired up in the desktop client.
    #[allow(dead_code)]
    Store { key: String, value: String },
    /// Delete a secret.
    Delete { key: String },
    /// Set access policy for a secret.
    SetPolicy { name: String, policy: String },
}

#[derive(Props, Clone, PartialEq)]
pub struct SecretsDialogProps {
    pub visible: bool,
    /// The canonical data from the gateway.
    pub data: SecretsDialogData,
    /// Called when the dialog needs the parent to perform an action.
    pub on_command: EventHandler<SecretsCommand>,
    /// Called when the dialog should be dismissed.
    pub on_close: EventHandler<()>,
}

/// Pre-computed fields for a single visible secret row, owned so the RSX
/// `for` loop can clone from them without `let` expressions.
#[derive(Clone)]
struct SecretRow {
    is_selected: bool,
    icon: &'static str,
    tone: rustyclaw_view::Tone,
    plabel: String,
    name: String,
    label: String,
    kind: String,
    policy: String,
}

#[component]
pub fn SecretsDialog(props: SecretsDialogProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    let d = &props.data;
    let sel_idx = d.selected.unwrap_or(0);

    // Pre-compute add-section display values (avoids `let` inside RSX branches).
    let add_step_data: Option<(String, String, String, &'static str)> = if d.is_adding() {
        let step_label = if d.add_step == 1 { "Name" } else { "Value" };
        let input_val = if d.add_step == 1 {
            d.add_name.clone()
        } else {
            d.add_value.clone()
        };
        let hint = if d.add_step == 1 {
            "Enter a name (e.g. openai_api_key), then press Enter for the value"
        } else {
            "Paste or type the value, then press Enter to save"
        };
        let input_type = if d.add_step == 2 { "password" } else { "text" };
        Some((
            step_label.to_string(),
            input_val,
            hint.to_string(),
            input_type,
        ))
    } else {
        None
    };

    // Pre-compute visible secret rows (avoids `let` inside RSX for loop).
    let rows: Vec<SecretRow> = {
        let end = (d.scroll_offset + 20).min(d.secrets.len());
        d.secrets[d.scroll_offset..end]
            .iter()
            .enumerate()
            .map(|(j, s)| {
                let idx = d.scroll_offset + j;
                SecretRow {
                    is_selected: idx == sel_idx,
                    icon: s.type_icon(),
                    tone: s.policy_tone(),
                    plabel: s.policy_label().to_string(),
                    name: s.key.clone(),
                    label: s.label.clone(),
                    kind: s.kind.clone(),
                    policy: s.policy.clone(),
                }
            })
            .collect()
    };

    // Build row VNodes outside the main RSX so closures can own their data.
    let is_empty = rows.is_empty();
    let row_elements: Vec<_> = rows
        .into_iter()
        .map(|row| {
            let name = row.name.clone();
            let policy = row.policy.clone();
            let icon = row.icon;
            let kind = row.kind.clone();
            let tone = row.tone;
            let plabel = row.plabel.clone();
            let label = row.label.clone();
            let is_selected = row.is_selected;

            rsx! {
                tr {
                    class: if is_selected { "secrets-row is-selected" } else { "secrets-row" },
                    key: "{name}",
                    td { class: "secrets-col-kind", "{icon} {kind}" }
                    td { class: "secrets-col-policy",
                        Tag {
                            color: tone_color(tone),
                            light: true,
                            rounded: true,
                            size: BulmaSize::Small,
                            "{plabel}"
                        }
                    }
                    td { class: "secrets-col-label", "{label}" }
                    td { class: "secrets-col-name", "{name}" }
                    td { class: "secrets-col-actions",
                        Button {
                            color: BulmaColor::Ghost,
                            size: BulmaSize::Small,
                            class: "secrets-policy-cycle",
                            onclick: {
                                let n = name.clone();
                                let p = policy.clone();
                                move |_| {
                                    props.on_command.call(
                                        SecretsCommand::SetPolicy {
                                            name: n.clone(),
                                            policy: next_policy(&p).to_string(),
                                        },
                                    );
                                }
                            },
                            "↻"
                        }
                        Delete {
                            size: BulmaSize::Small,
                            onclick: {
                                let n = name.clone();
                                move |_| {
                                    props.on_command.call(
                                        SecretsCommand::Delete { key: n.clone() },
                                    );
                                }
                            },
                        }
                    }
                }
            }
        })
        .collect();

    let access_label = if d.agent_access {
        "Enabled"
    } else {
        "Disabled"
    };
    let totp_label = if d.has_totp { "On" } else { "Off" };
    let is_adding = add_step_data.is_some();

    rsx! {
        RcModal {
            active: true,
            title: "🔐 Secrets Vault",
            width: 720,
            class: "secrets-dialog",
            onclose: move |_| props.on_close.call(()),
            footer: rsx! {
                Buttons { class: "secrets-foot",
                    Button {
                        color: BulmaColor::Light,
                        onclick: move |_| props.on_command.call(SecretsCommand::Refresh),
                        "↻ Refresh"
                    }
                    if is_adding {
                        Button {
                            color: BulmaColor::Ghost,
                            onclick: move |_| {
                                // Cancel add — parent resets add state
                            },
                            "Cancel"
                        }
                    } else {
                        Button {
                            color: BulmaColor::Primary,
                            outlined: true,
                            onclick: move |_| {
                                // Start add — parent sets add step
                            },
                            "+ Add Secret"
                        }
                    }
                    Button {
                        color: BulmaColor::Primary,
                        onclick: move |_| props.on_close.call(()),
                        "Done"
                    }
                }
            },

            // ── Status bar ──────────────────────────────────
            Level { class: "secrets-status",
                LevelLeft {
                    LevelItem { class: "secrets-status-item",
                        span { class: "secrets-status-label", "Agent Access:" }
                        Tag {
                            color: if d.agent_access { Some(BulmaColor::Success) } else { None },
                            light: true,
                            rounded: true,
                            "{access_label}"
                        }
                    }
                    LevelItem { class: "secrets-status-item",
                        span { class: "secrets-status-label", "Credentials:" }
                        span { "{d.secrets.len()}" }
                    }
                    LevelItem { class: "secrets-status-item",
                        span { class: "secrets-status-label", "2FA:" }
                        Tag {
                            color: if d.has_totp { Some(BulmaColor::Success) } else { None },
                            light: true,
                            rounded: true,
                            "{totp_label}"
                        }
                    }
                }
            }

            // ── Status message ──────────────────────────────
            if let Some(msg) = &d.status {
                div { class: "secrets-status-msg", "{msg}" }
            }

            // ── Credential list ─────────────────────────────
            if is_empty {
                div { class: "secrets-empty",
                    "No credentials stored in the vault."
                }
            } else {
                Table {
                    fullwidth: true,
                    hoverable: true,
                    narrow: true,
                    class: "secrets-table",
                    thead {
                        tr {
                            th { "Type" }
                            th { "Policy" }
                            th { "Label" }
                            th { "Name" }
                            th { "" }
                        }
                    }
                    tbody {
                        {row_elements.into_iter()}
                    }
                }
            }

            // ── Add-secret section ──────────────────────────
            if let Some((ref step_label, ref input_val, ref hint, input_type)) = add_step_data {
                div { class: "secrets-add-section",
                    div { class: "secrets-add-title", "Add Secret — {step_label}" }
                    input {
                        class: "input",
                        r#type: "{input_type}",
                        placeholder: if d.add_step == 1 { "e.g. openai_api_key" } else { "Paste secret value…" },
                        value: "{input_val}",
                        autofocus: true,
                    }
                    p { class: "help", "{hint}" }
                }
            }

            // ── Legend ──────────────────────────────────────
            if !is_adding {
                div { class: "secrets-legend",
                    for (policy, tone, meaning) in POLICY_LEGEND.iter() {
                        span { class: "secrets-legend-item",
                            Tag {
                                color: tone_color(*tone),
                                light: true,
                                rounded: true,
                                size: BulmaSize::Small,
                                "{policy}"
                            }
                            span { class: "secrets-legend-label", "{meaning}" }
                        }
                    }
                }
            }
        }
    }
}
