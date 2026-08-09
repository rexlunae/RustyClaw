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
        /// Whether the wake stays paused.
        ///
        /// The editor has no pause control — that lives on the row — so this
        /// is carried through unchanged rather than assumed. Sending `false`
        /// unconditionally resumed any paused wake the moment an unrelated
        /// detail was edited, and re-armed its schedule with it.
        paused: bool,
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

/// Whether the model has to be typed rather than picked.
///
/// True when the provider offers nothing to choose from: the local servers
/// (LM Studio, exo, llama.cpp, Joshua), the Copilot proxy and `custom` ship
/// no static catalogue, because they serve whatever the operator loaded.
/// With the model mandatory, a picker with no options is a form that cannot
/// be completed — typing the name is the only thing that was ever going to
/// work there.
///
/// Deliberately a question about the *provider*, not about the rendered
/// option list. The list also carries the value currently in the form, so
/// asking it would answer "not freeform" the moment the author typed a
/// single character, and the text field would collapse into a one-entry
/// menu mid-word.
fn model_is_freeform(provider_models: &HashMap<String, Vec<String>>, provider: &str) -> bool {
    models_for(provider_models, provider).is_empty()
}

/// The first field still standing between the author and a saveable wake,
/// named as the hint under the Save button reads it. `None` means ready.
///
/// Save is refused rather than defaulted for provider, model and thread
/// (#405, #406), which makes it possible to be stuck — so what is missing
/// has to be sayable, not just checkable.
fn missing_field(
    name: &str,
    expr: &str,
    prompt: &str,
    provider: &str,
    model: &str,
    thread_chosen: bool,
) -> Option<&'static str> {
    if name.trim().is_empty() {
        Some("a name")
    } else if expr.trim().is_empty() {
        Some("a schedule")
    } else if prompt.trim().is_empty() {
        Some("a prompt")
    } else if provider.trim().is_empty() {
        Some("a provider")
    } else if model.trim().is_empty() {
        Some("a model")
    } else if !thread_chosen {
        Some("where it runs")
    } else {
        None
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
    /// A provider was chosen in the editor. The app answers by asking the
    /// gateway for that provider's live model list — the standing fetch only
    /// covers the session's own provider, so without this the dialog would
    /// see nothing but the static catalogue for every other one.
    pub on_provider_pick: EventHandler<String>,
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
    // Carried, not edited: the editor offers no pause control, so whatever
    // the wake was set to has to survive a save of anything else.
    let mut form_paused = use_signal(|| false);

    if !props.visible {
        return rsx! {};
    }

    let is_editing = editing.read().is_some();
    let on_provider_pick = props.on_provider_pick;
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
    // Asked of the provider, not of the option list — and so, necessarily,
    // before the current value is folded in below.
    let model_is_freeform = model_is_freeform(&provider_models, &current_provider);
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

    let incomplete = missing_field(
        &form_name.read(),
        &form_expr.read(),
        &form_prompt.read(),
        &current_provider,
        &current_model,
        current_thread.is_some(),
    );

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
                paused: *form_paused.read(),
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
                                form_paused.set(false);
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
                                        // Ask for this provider's live list.
                                        // For the ones with no catalogue it
                                        // is the only way the picker ever
                                        // fills in; for the rest it replaces
                                        // a hardcoded list with the truth.
                                        on_provider_pick.call(picked.clone());
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
                            if model_is_freeform {
                                input {
                                    class: "input",
                                    r#type: "text",
                                    placeholder: "Model name as the server reports it",
                                    disabled: current_provider.is_empty(),
                                    value: "{form_model}",
                                    oninput: move |evt| form_model.set(evt.value()),
                                }
                                p { class: "help",
                                    if current_provider.is_empty() {
                                        "Choose a provider first."
                                    } else {
                                        "This provider has no model list yet — type the name it serves."
                                    }
                                }
                            } else {
                                div { class: "select is-fullwidth",
                                    select {
                                        value: "{form_model}",
                                        disabled: current_provider.is_empty(),
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
                            disabled: incomplete.is_some(),
                            onclick: on_save,
                            "Save"
                        }
                        dioxus_bulma::prelude::Button {
                            onclick: move |_| editing.set(None),
                            "Cancel"
                        }
                    }
                    if let Some(missing) = incomplete {
                        p { class: "help", "Still needs {missing}." }
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
                                                            let seeded = edit_seed.provider.clone().unwrap_or_default();
                                                            // Same request the picker makes, so
                                                            // opening an existing wake fills its
                                                            // model list too rather than only
                                                            // after the provider is re-picked.
                                                            if !seeded.is_empty() {
                                                                on_provider_pick.call(seeded.clone());
                                                            }
                                                            form_provider.set(seeded);
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
                                                            form_paused.set(edit_seed.paused);
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

    /// Six providers ship no static catalogue, because there is nothing to
    /// enumerate: the local servers and the proxies serve whatever the
    /// operator loaded. Making the model mandatory turned that into a form
    /// that could not be completed, which is why the editor falls back to a
    /// text field — so this is the precondition worth pinning, not the
    /// fallback itself.
    #[test]
    fn the_catalogue_less_providers_offer_nothing_to_pick() {
        let none = HashMap::new();
        for provider in [
            "copilot-proxy",
            "lmstudio",
            "exo",
            "llamacpp",
            "joshua",
            "custom",
        ] {
            assert!(
                models_for(&none, provider).is_empty(),
                "{provider} has a catalogue now — the freeform path may be unnecessary"
            );
            assert!(model_is_freeform(&none, provider));
        }
    }

    /// The text field must survive being typed into.
    ///
    /// It renders whenever the model is freeform, and the option list it sits
    /// beside also carries whatever is currently in the form. Deciding from
    /// that list rather than from the provider made the first keystroke turn
    /// the field into a one-entry menu, so only a paste of the whole name got
    /// through.
    #[test]
    fn a_half_typed_model_does_not_end_the_freeform_field() {
        let none = HashMap::new();
        for partial in ["q", "qwen3", "qwen3-coder-30b"] {
            assert!(
                model_is_freeform(&none, "lmstudio"),
                "typing {partial:?} must not turn the field back into a picker"
            );
            // What the render does with that partial value: it joins the
            // options so the field shows what is set. The freeform decision
            // must not be read back out of the result.
            let mut options = models_for(&none, "lmstudio");
            options.insert(0, partial.to_string());
            assert!(!options.is_empty());
        }
    }

    /// …and a provider that does publish a list still gets the picker.
    #[test]
    fn a_provider_with_a_catalogue_is_not_freeform() {
        assert!(!model_is_freeform(&HashMap::new(), "anthropic"));
        let mut live = HashMap::new();
        live.insert("lmstudio".to_string(), vec!["qwen3-coder-30b".to_string()]);
        assert!(
            !model_is_freeform(&live, "lmstudio"),
            "once the gateway answers, the picker should take over"
        );
    }

    /// …and that a live list from the gateway does reach them, which is the
    /// other half of the fix: the dialog asks for the picked provider's
    /// models rather than only the session's own.
    #[test]
    fn a_live_list_fills_in_a_provider_with_no_catalogue() {
        let mut live = HashMap::new();
        live.insert(
            "lmstudio".to_string(),
            vec!["qwen3-coder-30b".to_string(), "gemma-3-27b".to_string()],
        );
        assert_eq!(models_for(&live, "lmstudio").len(), 2);
        // An empty answer is not a list: fall back rather than showing a
        // picker with nothing in it.
        live.insert("exo".to_string(), vec![]);
        assert!(models_for(&live, "exo").is_empty());
    }

    #[test]
    fn the_missing_field_is_named_in_form_order() {
        assert_eq!(
            missing_field("", "", "", "", "", false),
            Some("a name"),
            "the first gap should be reported, not the last"
        );
        assert_eq!(
            missing_field("wake", " ", "do it", "anthropic", "claude-opus", true),
            Some("a schedule"),
            "whitespace is not a schedule"
        );
        assert_eq!(
            missing_field("wake", "every 1h", "", "anthropic", "claude-opus", true),
            Some("a prompt")
        );
        assert_eq!(
            missing_field("wake", "every 1h", "do it", "", "", true),
            Some("a provider")
        );
        assert_eq!(
            missing_field("wake", "every 1h", "do it", "lmstudio", "", true),
            Some("a model"),
            "a provider with no catalogue must still name the model as the gap"
        );
        // The thread is the one field where the empty string is a real
        // answer ("foreground"), so it is tracked as chosen-or-not.
        assert_eq!(
            missing_field("wake", "every 1h", "do it", "lmstudio", "qwen3", false),
            Some("where it runs")
        );
        assert_eq!(
            missing_field("wake", "every 1h", "do it", "lmstudio", "qwen3", true),
            None
        );
    }
}
