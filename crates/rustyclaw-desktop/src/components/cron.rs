//! Cron panel — scheduled wakes: view, edit, and fire jobs that run a
//! prompt as an agent turn on a schedule, into a chosen thread, with an
//! optional model override.

use std::collections::HashMap;

use dioxus::prelude::*;
use dioxus_bulma::prelude::BulmaColor;
use rustyclaw_core::gateway::CronActionKind;
use rustyclaw_core::providers;

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
        provider: Option<String>,
        model: Option<String>,
        thread_id: Option<u64>,
        /// The author picked "foreground" for a job that may currently be
        /// pinned. `thread_id: None` cannot carry that — it is also what a
        /// caller sends when it has nothing to say about the thread — so
        /// without this flag the pin would quietly outlive the choice.
        clear_thread: bool,
    },
}

/// Models to offer for a provider: the live list the gateway reported, or
/// the static catalogue when it has not answered for this one yet. Same
/// two-tier rule the composer's model bar uses, so the two pickers agree.
fn models_for(provider_models: &HashMap<String, Vec<String>>, provider: &str) -> Vec<String> {
    match provider_models.get(provider) {
        Some(live) if !live.is_empty() => live.clone(),
        _ => providers::models_for_provider(provider)
            .iter()
            .map(|m| (*m).to_string())
            .collect(),
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct CronDialogProps {
    pub visible: bool,
    pub data: Option<rustyclaw_view::CronPanelData>,
    /// `(id, label)` of the threads a wake can target, for the selector.
    pub threads: Vec<(u64, String)>,
    /// Live model lists per provider, as the composer's model bar uses.
    /// A provider missing here falls back to the static catalogue.
    pub provider_models: std::collections::HashMap<String, Vec<String>>,
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
    let mut form_provider = use_signal(String::new);
    let mut form_model = use_signal(String::new);
    // Distinct from the empty string, which is a legitimate choice
    // ("foreground"). `None` means the author has not chosen yet, and Save
    // refuses until they do — see #406: a wake landing in whatever thread
    // happens to be in front is rarely what was meant, and silently
    // defaulting to it hid the decision entirely.
    let mut form_thread = use_signal(|| Option::<String>::None);

    if !props.visible {
        return rsx! {};
    }

    let is_editing = editing.read().is_some();
    let threads = props.threads.clone();
    let provider_models = props.provider_models.clone();

    // A job saved against a provider or model this build no longer lists
    // still has to show what it is set to. Dropping it from the options
    // renders the picker blank, which reads as "nothing chosen" and turns
    // the next save into a silent re-point. Carry the current value at the
    // top instead — the same rule the composer's model bar follows.
    let current_provider = form_provider.read().clone();
    let current_model = form_model.read().clone();
    let mut provider_options: Vec<String> = providers::provider_ids()
        .iter()
        .map(|p| (*p).to_string())
        .collect();
    if !current_provider.is_empty() && !provider_options.contains(&current_provider) {
        provider_options.insert(0, current_provider.clone());
    }
    let mut model_options = models_for(&provider_models, &current_provider);
    if !current_model.is_empty() && !model_options.contains(&current_model) {
        model_options.insert(0, current_model.clone());
    }
    // Same for a thread that has since gone away: show the pin rather than
    // falling through to the empty value, which is "foreground".
    let current_thread = form_thread.read().clone();
    let orphan_thread = current_thread.as_deref().filter(|choice| {
        !choice.is_empty() && !threads.iter().any(|(id, _)| id.to_string() == *choice)
    });

    let on_save = {
        let on_command = props.on_command;
        move |_| {
            let name = form_name.read().trim().to_string();
            let expr = form_expr.read().trim().to_string();
            let prompt = form_prompt.read().trim().to_string();
            let provider = form_provider.read().trim().to_string();
            let model = form_model.read().trim().to_string();
            let thread_choice = form_thread.read().clone();
            // Model and provider join the required set (#405), and the thread
            // has to be chosen rather than defaulted (#406).
            if name.is_empty()
                || expr.is_empty()
                || prompt.is_empty()
                || provider.is_empty()
                || model.is_empty()
                || thread_choice.is_none()
            {
                return;
            }
            let thread_id = thread_choice.and_then(|t| t.trim().parse::<u64>().ok());
            let id = editing.read().clone().flatten();
            on_command.call(CronCommand::Save {
                id,
                name,
                expr,
                prompt,
                provider: Some(provider),
                model: Some(model),
                thread_id,
                // Save is gated on the author having chosen, so reaching
                // here with no thread means they chose the foreground.
                clear_thread: thread_id.is_none(),
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
                                form_provider.set(String::new());
                                form_model.set(String::new());
                                form_thread.set(None);
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
                            label { class: "label is-small", "Provider" }
                            div { class: "select is-fullwidth",
                                select {
                                    value: "{form_provider}",
                                    onchange: move |evt| {
                                        let picked = evt.value();
                                        // Keep the model if the new provider
                                        // also offers it; otherwise take its
                                        // first, so the pair is never a
                                        // combination that cannot run.
                                        let models = models_for(&provider_models, &picked);
                                        let keep = models.iter().any(|m| *m == *form_model.read());
                                        if !keep {
                                            form_model.set(
                                                models.first().cloned().unwrap_or_default(),
                                            );
                                        }
                                        form_provider.set(picked);
                                    },
                                    option { value: "", disabled: true, "Select provider" }
                                    for id in provider_options.iter() {
                                        option {
                                            key: "{id}",
                                            value: "{id}",
                                            "{providers::display_name_for_provider(id)}"
                                        }
                                    }
                                }
                            }
                        }
                        div { class: "column",
                            label { class: "label is-small", "Model" }
                            div { class: "select is-fullwidth",
                                select {
                                    value: "{form_model}",
                                    disabled: form_provider.read().is_empty(),
                                    onchange: move |evt| form_model.set(evt.value()),
                                    option { value: "", disabled: true, "Select model" }
                                    for name in model_options.iter() {
                                        option {
                                            key: "{name}",
                                            value: "{name}",
                                            "{name}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div { class: "field",
                        label { class: "label is-small", "Thread" }
                        div { class: "select is-fullwidth",
                            select {
                                value: "{form_thread.read().clone().unwrap_or_default()}",
                                onchange: move |evt| form_thread.set(Some(evt.value())),
                                // Unchosen is its own state, distinct from
                                // "foreground": a wake landing wherever the
                                // user happens to be looking is a decision,
                                // not a default.
                                if form_thread.read().is_none() {
                                    option { value: "", disabled: true, "Select where it runs" }
                                }
                                option { value: "", "Foreground at fire time" }
                                if let Some(gone) = orphan_thread {
                                    option { value: "{gone}", "#{gone} — (no longer listed)" }
                                }
                                for (id, label) in threads.iter() {
                                    option {
                                        key: "{id}",
                                        value: "{id}",
                                        "#{id} — {label}"
                                    }
                                }
                            }
                        }
                        p { class: "help",
                            "Foreground follows whichever thread is in front when it fires; a named thread is fixed."
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
                                                            form_provider.set(edit_seed.provider.clone().unwrap_or_default());
                                                            form_model.set(edit_seed.model.clone().unwrap_or_default());
                                                            // An existing job already expresses a
                                                            // thread choice, including "foreground"
                                                            // as the empty string, so editing one
                                                            // starts chosen rather than blank.
                                                            form_thread.set(Some(
                                                                edit_seed
                                                                    .thread_id
                                                                    .map(|t| t.to_string())
                                                                    .unwrap_or_default(),
                                                            ));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_live_model_list_wins_over_the_static_catalogue() {
        let mut live = HashMap::new();
        live.insert(
            "openai".to_string(),
            vec!["gpt-5-turbo".to_string(), "gpt-5-mini".to_string()],
        );
        assert_eq!(
            models_for(&live, "openai"),
            vec!["gpt-5-turbo".to_string(), "gpt-5-mini".to_string()]
        );
    }

    #[test]
    fn an_empty_live_list_falls_back_rather_than_offering_nothing() {
        // The gateway answers with an empty list for a provider it could not
        // reach. Showing no models at all would make the field unfillable and
        // the form unsubmittable, so the static catalogue stands in.
        let mut live = HashMap::new();
        live.insert("openai".to_string(), Vec::new());
        assert_eq!(
            models_for(&live, "openai"),
            models_for(&HashMap::new(), "openai"),
            "an empty live list is the same as no live list"
        );
    }

    #[test]
    fn a_provider_with_no_live_list_uses_the_catalogue() {
        let models = models_for(&HashMap::new(), "openai");
        assert!(!models.is_empty(), "openai must offer something to pick");
    }

    #[test]
    fn an_unknown_provider_offers_nothing() {
        assert!(models_for(&HashMap::new(), "not-a-provider").is_empty());
    }
}
