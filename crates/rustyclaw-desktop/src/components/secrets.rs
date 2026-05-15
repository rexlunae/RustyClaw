//! Secrets management dialog — view and manage vault credentials.
//!
//! Displays all secrets stored in the gateway vault and emits commands
//! for refresh, store, delete, and set-policy operations.  The parent
//! component manages all mutable state.

use dioxus::prelude::*;
use rustyclaw_view::SecretsDialogData;

/// Commands emitted by the secrets dialog back to the parent.
#[derive(Clone, Debug)]
pub enum SecretsCommand {
    /// Request secrets list refresh from gateway.
    Refresh,
    /// Store a new secret (used by add-secret flow).
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

// ── Helpers ─────────────────────────────────────────────────────────────────

fn policy_class(policy: &str, disabled: bool) -> &'static str {
    if disabled {
        "badge-muted"
    } else {
        match policy {
            "OPEN" => "badge-open",
            "ASK" => "badge-ask",
            "AUTH" => "badge-auth",
            "SKILL" => "badge-skill",
            _ => "badge-muted",
        }
    }
}

fn next_policy(current: &str) -> &'static str {
    match current {
        "OPEN" => "ASK",
        "ASK" => "AUTH",
        "AUTH" => "SKILL",
        "SKILL" => "OPEN",
        _ => "OPEN",
    }
}

/// Pre-computed fields for a single visible secret row, owned so the RSX
/// `for` loop can clone from them without `let` expressions.
#[derive(Clone)]
struct SecretRow {
    is_selected: bool,
    icon: &'static str,
    pclass: &'static str,
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
        Some((step_label.to_string(), input_val, hint.to_string(), input_type))
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
                    pclass: policy_class(&s.policy, s.disabled),
                    plabel: if s.disabled { "OFF".into() } else { s.policy.clone() },
                    name: s.key.clone(),
                    label: s.label.clone(),
                    kind: s.kind.clone(),
                    policy: s.policy.clone(),
                }
            })
            .collect()
    };

    // Build row VNodes outside the main RSX so closures can own their data.
    // Each closure clones what it needs (name, policy) and does not borrow
    // the local `rows` vector, satisfying the 'static requirement.
    let is_empty = rows.is_empty();
    let has_any = !d.secrets.is_empty();
    let row_elements: Vec<_> = rows.into_iter().map(|row| {
        let name = row.name.clone();
        let policy = row.policy.clone();
        let icon = row.icon;
        let kind = row.kind.clone();
        let pclass = row.pclass;
        let plabel = row.plabel.clone();
        let label = row.label.clone();
        let is_selected = row.is_selected;

        rsx! {
            div {
                class: if is_selected { "secrets-row selected" } else { "secrets-row" },
                key: "{name}",
                span { class: "secrets-col-kind", "{icon} {kind}" }
                span { class: "secrets-col-policy",
                    span { class: "badge {pclass}", "{plabel}" }
                }
                span { class: "secrets-col-label", "{label}" }
                span { class: "secrets-col-name", "{name}" }
                span { class: "secrets-col-actions",
                    button {
                        class: "btn btn-ghost btn-xs",
                        title: "Cycle policy (OPEN ↔ ASK ↔ AUTH ↔ SKILL)",
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
                    button {
                        class: "btn btn-ghost btn-xs btn-danger-hover",
                        title: "Delete secret",
                        onclick: {
                            let n = name.clone();
                            move |_| {
                                props.on_command.call(
                                    SecretsCommand::Delete { key: n.clone() },
                                );
                            }
                        },
                        "\u{2715}"
                    }
                }
            }
        }
    }).collect();

    let access_label = if d.agent_access { "Enabled" } else { "Disabled" };
    let totp_label = if d.has_totp { "On" } else { "Off" };

    rsx! {
        div {
            class: "modal-backdrop",
            onclick: move |_| props.on_close.call(()),

            div {
                class: "modal secrets-dialog",
                onclick: move |evt| evt.stop_propagation(),

                // ── Header ──────────────────────────────────────────
                div { class: "modal-head",
                    span { class: "modal-title", "🔐 Secrets Vault" }
                    button {
                        class: "modal-close",
                        title: "Close",
                        onclick: move |_| props.on_close.call(()),
                        "\u{2715}"
                    }
                }

                div { class: "modal-body",
                    // ── Status bar ──────────────────────────────────
                    div { class: "secrets-status",
                        span { class: "secrets-status-item",
                            span { class: "secrets-status-label", "Agent Access:" }
                            span {
                                class: if d.agent_access { "badge-open" } else { "badge-muted" },
                                "{access_label}"
                            }
                        }
                        span { class: "secrets-status-item",
                            span { class: "secrets-status-label", "Credentials:" }
                            span { "{d.secrets.len()}" }
                        }
                        span { class: "secrets-status-item",
                            span { class: "secrets-status-label", "2FA:" }
                            span {
                                class: if d.has_totp { "badge-open" } else { "badge-muted" },
                                "{totp_label}"
                            }
                        }
                    }

                    // ── Status message ──────────────────────────────
                    if let Some(msg) = &d.status {
                        div { class: "secrets-status-msg", "{msg}" }
                    }

                    // ── Credential list ─────────────────────────────
                    div { class: "secrets-list",
                        if has_any && !is_empty {
                            div { class: "secrets-list-header",
                                span { class: "secrets-col-kind", "Type" }
                                span { class: "secrets-col-policy", "Policy" }
                                span { class: "secrets-col-label", "Label" }
                                span { class: "secrets-col-name", "Name" }
                                span { class: "secrets-col-actions", "" }
                            }
                        }

                        if is_empty {
                            div { class: "secrets-empty",
                                "No credentials stored in the vault."
                            }
                        } else {
                            {row_elements.into_iter()}
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
                            span { class: "field-help", "{hint}" }
                        }
                    }

                    // ── Legend ──────────────────────────────────────
                    if add_step_data.is_none() {
                        div { class: "secrets-legend",
                            span { class: "secrets-legend-item",
                                span { class: "badge badge-open", "OPEN" }
                            }
                            span { class: "secrets-legend-label", "anytime" }
                            span { class: "secrets-legend-item",
                                span { class: "badge badge-ask", "ASK" }
                            }
                            span { class: "secrets-legend-label", "per-use" }
                            span { class: "secrets-legend-item",
                                span { class: "badge badge-auth", "AUTH" }
                            }
                            span { class: "secrets-legend-label", "re-auth" }
                            span { class: "secrets-legend-item",
                                span { class: "badge badge-skill", "SKILL" }
                            }
                            span { class: "secrets-legend-label", "gated" }
                            span { class: "secrets-legend-item",
                                span { class: "badge badge-muted", "OFF" }
                            }
                            span { class: "secrets-legend-label", "disabled" }
                        }
                    }
                }

                // ── Footer ──────────────────────────────────────────
                div { class: "modal-foot secrets-foot",
                    button {
                        class: "btn btn-subtle",
                        onclick: move |_| props.on_command.call(SecretsCommand::Refresh),
                        "↻ Refresh"
                    }
                    if add_step_data.is_some() {
                        button {
                            class: "btn btn-ghost",
                            onclick: move |_| {
                                // Cancel add — parent resets add state
                            },
                            "Cancel"
                        }
                    } else {
                        button {
                            class: "btn btn-primary",
                            onclick: move |_| {
                                // Start add — parent sets add step
                            },
                            "+ Add Secret"
                        }
                    }
                    button {
                        class: "btn btn-primary",
                        onclick: move |_| props.on_close.call(()),
                        "Done"
                    }
                }
            }
        }
    }
}
