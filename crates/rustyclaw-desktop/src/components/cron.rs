//! Cron panel — scheduled wakes: view, edit, and fire jobs that run a
//! prompt as an agent turn on a schedule, into a chosen thread, with an
//! optional model override.

use dioxus::prelude::*;
use dioxus_bulma::prelude::BulmaColor;
use rustyclaw_core::gateway::CronActionKind;

use super::RcModal;

/// What the dialog asks the app to do. The mount site translates these
/// into gateway commands (and follows each with a list refresh).
#[derive(Clone, Debug, PartialEq)]
pub enum CronCommand {
    Action {
        id: String,
        action: CronActionKind,
    },
    Save {
        id: Option<String>,
        name: String,
        expr: String,
        prompt: String,
        model: Option<String>,
        thread_id: Option<u64>,
    },
}

#[derive(Props, Clone, PartialEq)]
pub struct CronDialogProps {
    pub visible: bool,
    pub data: Option<rustyclaw_view::CronPanelData>,
    /// `(id, label)` of the threads a wake can target, for the selector.
    pub threads: Vec<(u64, String)>,
    pub on_close: EventHandler<()>,
    pub on_command: EventHandler<CronCommand>,
}

#[component]
pub fn CronDialog(props: CronDialogProps) -> Element {
    // The editor form. `editing` is None when closed, Some(None) for a new
    // job, Some(Some(id)) when editing an existing one.
    let mut editing = use_signal(|| Option::<Option<String>>::None);
    let mut form_name = use_signal(String::new);
    let mut form_expr = use_signal(String::new);
    let mut form_prompt = use_signal(String::new);
    let mut form_model = use_signal(String::new);
    let mut form_thread = use_signal(String::new);

    if !props.visible {
        return rsx! {};
    }

    let is_editing = editing.read().is_some();
    let threads = props.threads.clone();

    let on_save = {
        let on_command = props.on_command;
        move |_| {
            let name = form_name.read().trim().to_string();
            let expr = form_expr.read().trim().to_string();
            let prompt = form_prompt.read().trim().to_string();
            if name.is_empty() || expr.is_empty() || prompt.is_empty() {
                return;
            }
            let model = {
                let m = form_model.read().trim().to_string();
                if m.is_empty() { None } else { Some(m) }
            };
            let thread_id = form_thread.read().trim().parse::<u64>().ok();
            let id = editing.read().clone().flatten();
            on_command.call(CronCommand::Save {
                id,
                name,
                expr,
                prompt,
                model,
                thread_id,
            });
            editing.set(None);
        }
    };

    rsx! {
        RcModal {
            active: true,
            title: "Scheduled Wakes",
            width: 760,
            onclose: move |_| props.on_close.call(()),
            footer: rsx! {
                dioxus_bulma::prelude::Buttons {
                    if !is_editing {
                        dioxus_bulma::prelude::Button {
                            color: BulmaColor::Link,
                            onclick: move |_| {
                                form_name.set(String::new());
                                form_expr.set(String::new());
                                form_prompt.set(String::new());
                                form_model.set(String::new());
                                form_thread.set(String::new());
                                editing.set(Some(None));
                            },
                            "New wake"
                        }
                    }
                    dioxus_bulma::prelude::Button {
                        color: BulmaColor::Primary,
                        onclick: move |_| props.on_close.call(()),
                        "Close"
                    }
                }
            },

            if is_editing {
                div { class: "box",
                    div { class: "field",
                        label { class: "label is-small", "Name" }
                        input {
                            class: "input",
                            r#type: "text",
                            placeholder: "Morning check-in",
                            value: "{form_name}",
                            autofocus: true,
                            oninput: move |evt| form_name.set(evt.value()),
                        }
                    }
                    div { class: "field",
                        label { class: "label is-small", "Schedule" }
                        input {
                            class: "input",
                            r#type: "text",
                            placeholder: "every 1h · at 2026-01-01T09:00:00Z · 0 9 * * 1-5",
                            value: "{form_expr}",
                            oninput: move |evt| form_expr.set(evt.value()),
                        }
                        p { class: "help",
                            "'at <ISO-8601>' once · 'every <N>[s|m|h]' interval · 5-field cron expression"
                        }
                    }
                    div { class: "field",
                        label { class: "label is-small", "Prompt" }
                        textarea {
                            class: "textarea",
                            rows: 3,
                            placeholder: "What the agent wakes up to do…",
                            value: "{form_prompt}",
                            oninput: move |evt| form_prompt.set(evt.value()),
                        }
                    }
                    div { class: "columns",
                        div { class: "column",
                            label { class: "label is-small", "Model (optional)" }
                            input {
                                class: "input",
                                r#type: "text",
                                placeholder: "gateway default",
                                value: "{form_model}",
                                oninput: move |evt| form_model.set(evt.value()),
                            }
                        }
                        div { class: "column",
                            label { class: "label is-small", "Thread" }
                            div { class: "select is-fullwidth",
                                select {
                                    value: "{form_thread}",
                                    onchange: move |evt| form_thread.set(evt.value()),
                                    option { value: "", "Foreground at fire time" }
                                    for (id, label) in threads.iter() {
                                        option {
                                            key: "{id}",
                                            value: "{id}",
                                            "#{id} — {label}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                    dioxus_bulma::prelude::Buttons {
                        dioxus_bulma::prelude::Button {
                            color: BulmaColor::Success,
                            onclick: on_save,
                            "Save"
                        }
                        dioxus_bulma::prelude::Button {
                            onclick: move |_| editing.set(None),
                            "Cancel"
                        }
                    }
                }
            } else if let Some(ref data) = props.data {
                if data.jobs.is_empty() {
                    p { class: "has-text-grey", "No scheduled wakes. Create one with \"New wake\"." }
                } else {
                    div { class: "mb-3",
                        span { class: "tag is-info is-light",
                            "{data.active_count()} active / {data.total_count()} total"
                        }
                    }

                    table { class: "table is-narrow is-fullwidth is-hoverable",
                        thead {
                            tr {
                                th { "Name" }
                                th { "Schedule" }
                                th { "Status" }
                                th { "Next Run" }
                                th { "Target" }
                                th { "" }
                            }
                        }
                        tbody {
                            for job in data.jobs.iter() {
                                {
                                    let id_pause = job.id.clone();
                                    let id_run = job.id.clone();
                                    let id_del = job.id.clone();
                                    let paused = job.paused;
                                    let edit_seed = job.clone();
                                    let on_command = props.on_command;
                                    rsx! {
                                        tr { key: "{job.id}",
                                            td {
                                                strong { "{job.name}" }
                                                if let Some(ref model) = job.model {
                                                    p { class: "help", "model: {model}" }
                                                }
                                            }
                                            td { code { "{job.expr}" } }
                                            td { span { class: "tag", "{job.status_label()}" } }
                                            td {
                                                if let Some(ref next) = job.next_run {
                                                    "{next}"
                                                } else {
                                                    "—"
                                                }
                                            }
                                            td {
                                                if let Some(thread) = job.thread_id {
                                                    "thread #{thread}"
                                                } else {
                                                    "foreground"
                                                }
                                            }
                                            td {
                                                div { class: "buttons are-small",
                                                    button {
                                                        class: "button is-small",
                                                        onclick: move |_| {
                                                            form_name.set(edit_seed.name.clone());
                                                            form_expr.set(edit_seed.expr.clone());
                                                            form_prompt.set(edit_seed.payload.clone());
                                                            form_model.set(edit_seed.model.clone().unwrap_or_default());
                                                            form_thread.set(
                                                                edit_seed
                                                                    .thread_id
                                                                    .map(|t| t.to_string())
                                                                    .unwrap_or_default(),
                                                            );
                                                            editing.set(Some(Some(edit_seed.id.clone())));
                                                        },
                                                        "Edit"
                                                    }
                                                    button {
                                                        class: "button is-small",
                                                        onclick: move |_| on_command.call(CronCommand::Action {
                                                            id: id_pause.clone(),
                                                            action: if paused {
                                                                CronActionKind::Resume
                                                            } else {
                                                                CronActionKind::Pause
                                                            },
                                                        }),
                                                        if paused { "Resume" } else { "Pause" }
                                                    }
                                                    button {
                                                        class: "button is-small",
                                                        onclick: move |_| on_command.call(CronCommand::Action {
                                                            id: id_run.clone(),
                                                            action: CronActionKind::Run,
                                                        }),
                                                        "Run"
                                                    }
                                                    button {
                                                        class: "button is-small is-danger is-outlined",
                                                        onclick: move |_| on_command.call(CronCommand::Action {
                                                            id: id_del.clone(),
                                                            action: CronActionKind::Remove,
                                                        }),
                                                        "Delete"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                p { class: "has-text-grey", "Cron system not initialised." }
            }
        }
    }
}
