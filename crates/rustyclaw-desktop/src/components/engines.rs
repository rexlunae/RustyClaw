//! Engines panel — local engine and model management dialog.
//!
//! Laid out as one tab per detected engine: the tab strip switches the
//! active engine, and the body shows that engine's status, actions, models,
//! live install output, and any pull progress.  Each engine tab also carries
//! a parameters editor (context window, device, huge pages, …) whose values
//! are persisted through `EngineConfigSet` and applied on the next
//! start/load.

use dioxus::prelude::*;
use dioxus_bulma::prelude::BulmaColor;
use rustyclaw_core::engines::EngineConfig;
use rustyclaw_core::gateway::{EngineActionKind, ModelActionKind};

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct EnginesDialogProps {
    pub visible: bool,
    pub data: Option<rustyclaw_view::EnginesPanelData>,
    /// (engine, model) whose load/unload action is in flight; the matching
    /// row's button shows "Loading…" and is disabled.
    pub action_pending: Option<(String, String)>,
    /// Outcome of the last engine model action (engine, ok, message),
    /// rendered as an inline alert on that engine's tab.
    pub action_result: Option<(String, bool, String)>,
    /// Dismiss the inline action result (its alert's close button).
    pub on_clear_action_result: EventHandler<()>,
    pub on_close: EventHandler<()>,
    pub on_engine_action: EventHandler<(String, EngineActionKind)>,
    pub on_model_action: EventHandler<(String, String, ModelActionKind)>,
    pub on_pull: EventHandler<(String, String)>,
    /// Select an engine to browse its models (sends a model-list request).
    /// Also used by the tab strip to switch the active engine.
    pub on_select_engine: EventHandler<String>,
    /// Switch the active chat provider/model to this local (engine, model).
    pub on_use_model: EventHandler<(String, String)>,
    /// Save the full configuration for an engine (parameters, default model,
    /// auto-start, extra args) — sent as `EngineConfigSet`.
    pub on_config_save: EventHandler<(String, EngineConfig)>,
    /// Whether the gateway's `EngineConfigList` snapshot has arrived.  Until
    /// it does, the engine configs shown here are placeholders and saving
    /// would overwrite the real settings with blanks — the Save button is
    /// disabled instead.
    pub configs_received: bool,
    /// Re-fetch the engine list (and selected engine's models).
    pub on_refresh: EventHandler<()>,
}

/// Editable parameter form for one engine, seeded from its config.  Kept in
/// a map keyed by engine id so switching tabs doesn't lose in-progress edits,
/// and re-seeded whenever the gateway reports a different config (i.e. after
/// a save round-trip or an external config change).
#[derive(Clone, PartialEq)]
struct EngineParamsForm {
    /// Config this form was seeded from — when it differs, the form re-seeds.
    config_seen: EngineConfig,
    context_length: String,
    device: String,
    huge_pages: String,
    mmap: bool,
    lazy_weights: bool,
    max_output_tokens: String,
    max_concurrency: String,
    default_model: String,
    auto_start: bool,
}

impl EngineParamsForm {
    fn from_config(cfg: &EngineConfig) -> Self {
        Self {
            config_seen: cfg.clone(),
            context_length: cfg
                .context_length
                .map(|v| v.to_string())
                .unwrap_or_default(),
            device: cfg.device.clone().unwrap_or_default(),
            huge_pages: cfg.huge_pages.clone().unwrap_or_default(),
            mmap: cfg.mmap,
            lazy_weights: cfg.lazy_weights,
            max_output_tokens: cfg
                .max_output_tokens
                .map(|v| v.to_string())
                .unwrap_or_default(),
            max_concurrency: cfg
                .max_concurrency
                .map(|v| v.to_string())
                .unwrap_or_default(),
            default_model: cfg.default_model.clone().unwrap_or_default(),
            auto_start: cfg.auto_start,
        }
    }

    fn apply_to(&self, cfg: &mut EngineConfig) {
        cfg.context_length = parse_opt_u32(&self.context_length);
        cfg.device = opt_string(&self.device);
        cfg.huge_pages = opt_string(&self.huge_pages);
        cfg.mmap = self.mmap;
        cfg.lazy_weights = self.lazy_weights;
        cfg.max_output_tokens = parse_opt_u32(&self.max_output_tokens);
        cfg.max_concurrency = parse_opt_u32(&self.max_concurrency);
        cfg.default_model = opt_string(&self.default_model);
        cfg.auto_start = self.auto_start;
    }
}

fn parse_opt_u32(raw: &str) -> Option<u32> {
    let raw = raw.trim();
    if raw.is_empty() {
        None
    } else {
        raw.parse().ok()
    }
}

fn opt_string(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        None
    } else {
        Some(raw.to_string())
    }
}

/// The form to display for an engine: the in-progress edit when it was
/// seeded from the config the gateway currently reports, otherwise a fresh
/// form built from that config.  A form whose `config_seen` no longer
/// matches is stale (the config changed under it — e.g. after a save) and
/// is ignored in favour of the fresh config.
fn params_form_for(
    params_form: Signal<std::collections::HashMap<String, EngineParamsForm>>,
    eid: &str,
    fallback: &EngineConfig,
) -> EngineParamsForm {
    params_form
        .read()
        .get(eid)
        .cloned()
        .filter(|f| f.config_seen == *fallback)
        .unwrap_or_else(|| EngineParamsForm::from_config(fallback))
}

/// Apply `edit` to the engine's form, creating it from `fallback` when
/// missing and re-seeding it when the config changed under it (so in-progress
/// edits never apply to a stale base).
fn params_set(
    mut params_form: Signal<std::collections::HashMap<String, EngineParamsForm>>,
    eid: &str,
    fallback: &EngineConfig,
    edit: impl FnOnce(&mut EngineParamsForm),
) {
    let mut map = params_form.write();
    let entry = map
        .entry(eid.to_string())
        .or_insert_with(|| EngineParamsForm::from_config(fallback));
    if entry.config_seen != *fallback {
        *entry = EngineParamsForm::from_config(fallback);
    }
    edit(entry);
}

#[component]
pub fn EnginesDialog(props: EnginesDialogProps) -> Element {
    let mut pull_input = use_signal(String::new);
    let params_form = use_signal(std::collections::HashMap::<String, EngineParamsForm>::new);

    if !props.visible {
        return rsx! {};
    }

    rsx! {
        RcModal {
            active: true,
            title: "Local Engines & Models",
            width: 800,
            onclose: move |_| props.on_close.call(()),
            footer: rsx! {
                dioxus_bulma::prelude::Buttons {
                    dioxus_bulma::prelude::Button {
                        onclick: move |_| props.on_refresh.call(()),
                        "Refresh"
                    }
                    dioxus_bulma::prelude::Button {
                        color: BulmaColor::Primary,
                        onclick: move |_| props.on_close.call(()),
                        "Close"
                    }
                }
            },
            if let Some(ref data) = props.data {
                // Resource header (shared context above the tabs).
                if data.host_ram_bytes > 0 || data.host_vram_bytes > 0 {
                    div { class: "notification is-info is-light mb-4",
                        strong { "Host: " }
                        span {
                            "RAM: {format_bytes(data.host_ram_bytes)} | "
                            "VRAM: {format_bytes(data.host_vram_bytes)}"
                        }
                        if let Some(ref gpu) = data.host_gpu_name {
                            span { " ({gpu})" }
                        }
                    }
                }

                if data.engines.is_empty() {
                    p { class: "has-text-grey", "(no engines detected)" }
                } else {
                    // ── Tab strip: one tab per engine ────────────────────
                    div { class: "tabs is-boxed",
                        ul {
                            for engine in data.engines.iter() {
                                {
                                    let eid = engine.id.clone();
                                    let active = data.active_engine().map(|e| e.id.as_str())
                                        == Some(engine.id.as_str());
                                    rsx! {
                                        li {
                                            class: if active { "is-active" } else { "" },
                                            a {
                                                onclick: move |_| props.on_select_engine.call(eid.clone()),
                                                span { "{engine.display_name}" }
                                                span {
                                                    class: "tag {engine.status_class()} ml-2",
                                                    "{engine.status_badge()}"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // ── Active engine body ───────────────────────────────
                    if let Some(engine) = data.active_engine() {
                        div { class: "box mb-3",
                            div { class: "level",
                                div { class: "level-left",
                                    div { class: "level-item",
                                        strong { "{engine.display_name}" }
                                    }
                                    div { class: "level-item",
                                        span { class: "tag {engine.status_class()}",
                                            "{engine.status_badge()}"
                                        }
                                    }
                                    if let Some(ref ver) = engine.version {
                                        div { class: "level-item",
                                            span { class: "is-size-7 has-text-grey", "v{ver}" }
                                        }
                                    }
                                }
                                div { class: "level-right",
                                    if !engine.installed && engine.can("install") {
                                        div { class: "level-item",
                                            {
                                                let eid = engine.id.clone();
                                                let installing = data
                                                    .install_output
                                                    .get(&engine.id)
                                                    .is_some_and(|o| !o.done);
                                                rsx! {
                                                    dioxus_bulma::prelude::Button {
                                                        color: BulmaColor::Info,
                                                        disabled: installing,
                                                        onclick: move |_| props.on_engine_action.call((eid.clone(), EngineActionKind::Install)),
                                                        if installing { "Installing…" } else { "Install" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    if engine.installed && !engine.running && engine.can("start") {
                                        div { class: "level-item",
                                            {
                                                let eid = engine.id.clone();
                                                rsx! {
                                                    dioxus_bulma::prelude::Button {
                                                        color: BulmaColor::Success,
                                                        onclick: move |_| props.on_engine_action.call((eid.clone(), EngineActionKind::Start)),
                                                        "Start"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    if engine.running && engine.can("stop") {
                                        div { class: "level-item",
                                            {
                                                let eid = engine.id.clone();
                                                rsx! {
                                                    dioxus_bulma::prelude::Button {
                                                        color: BulmaColor::Warning,
                                                        onclick: move |_| props.on_engine_action.call((eid.clone(), EngineActionKind::Stop)),
                                                        "Stop"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    if engine.running && engine.can("stop") && engine.can("start") {
                                        // Restart applies the saved parameters
                                        // without a manual Stop then Start.
                                        div { class: "level-item",
                                            {
                                                let eid = engine.id.clone();
                                                let on_engine_action = props.on_engine_action;
                                                rsx! {
                                                    dioxus_bulma::prelude::Button {
                                                        color: BulmaColor::Link,
                                                        onclick: move |_| {
                                                            on_engine_action.call((eid.clone(), EngineActionKind::Stop));
                                                            on_engine_action.call((eid.clone(), EngineActionKind::Start));
                                                        },
                                                        "Restart"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    if engine.running {
                                        div { class: "level-item",
                                            {
                                                let eid = engine.id.clone();
                                                rsx! {
                                                    dioxus_bulma::prelude::Button {
                                                        color: BulmaColor::Link,
                                                        onclick: move |_| props.on_select_engine.call(eid.clone()),
                                                        "Refresh models"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            if engine.running {
                                p { class: "is-size-7 has-text-grey",
                                    "{engine.available_models} model(s) available, {engine.loaded_models} loaded"
                                }
                                if let Some(ref ep) = engine.endpoint {
                                    p { class: "is-size-7 has-text-grey", "Endpoint: {ep}" }
                                }
                            }
                        }

                        // ── Inline action result ──────────────────────────
                        // The outcome of the last Load/Unload on this
                        // engine, so clicking a button always answers.
                        if let Some((result_engine, ok, message)) = &props.action_result {
                            if result_engine == &engine.id {
                                div {
                                    class: if *ok { "notification is-success is-light" }
                                        else { "notification is-danger is-light" },
                                    button {
                                        class: "delete",
                                        onclick: move |_| props.on_clear_action_result.call(()),
                                    }
                                    p { "{message}" }
                                }
                            }
                        }

                        // ── Parameters editor ───────────────────────────
                        // Shown for engines that can start (startup settings
                        // apply) or that expose model parameters.
                        if engine.can("start")
                            || engine.supports_context_length()
                            || engine.supports_joshua_parameters()
                            || engine.supports_default_model()
                        {
                            {
                                let eid = engine.id.clone();
                                // Owned copies for 'static closures (the
                                // dialog's event handlers cannot borrow
                                // props).
                                let fallback_config = engine.config.clone();
                                let form = params_form_for(params_form, &eid, &fallback_config);
                                let params_form_handle = params_form;
                                let on_config_save = props.on_config_save;
                                let model_names: Vec<String> =
                                    data.models.iter().map(|m| m.name.clone()).collect();
                                rsx! {
                                    div { class: "box mb-3",
                                        div { class: "level",
                                            div { class: "level-left",
                                                div { class: "level-item",
                                                    h5 { class: "title is-5 mb-0", "Parameters" }
                                                }
                                            }
                                            div { class: "level-right",
                                                div { class: "level-item",
                                                    dioxus_bulma::prelude::Button {
                                                        color: BulmaColor::Primary,
                                                        size: dioxus_bulma::prelude::BulmaSize::Small,
                                                        // Until the gateway's config snapshot arrives, the
                                                        // base config here is a placeholder: saving would
                                                        // blank out enabled/endpoint/port/models_dir/
                                                        // extra_args, so the button stays disabled.
                                                        disabled: !props.configs_received,
                                                        onclick: {
                                                            let eid_save = eid.clone();
                                                            let fallback = fallback_config.clone();
                                                            move |_| {
                                                                // Rebuild the full config: start from the
                                                                // config the gateway last reported (which
                                                                // preserves enabled/endpoint/port/models_dir/
                                                                // extra_args) and overlay the edited fields.
                                                                let form = params_form_for(
                                                                    params_form_handle,
                                                                    &eid_save,
                                                                    &fallback,
                                                                );
                                                                let mut cfg = fallback.clone();
                                                                form.apply_to(&mut cfg);
                                                                on_config_save.call((eid_save.clone(), cfg));
                                                            }
                                                        },
                                                        "Save parameters"
                                                    }
                                                }
                                            }
                                        }
                                        p { class: "is-size-7 has-text-grey mb-3",
                                            if props.configs_received {
                                                "Applied on the next Start or model Load."
                                            } else {
                                                "Loading engine settings from the gateway… (saving disabled until they arrive)"
                                            }
                                        }
                                        div { class: "columns is-multiline is-variable is-2",
                                            if engine.supports_context_length() {
                                                div { class: "column is-half",
                                                    label { class: "label is-size-7", "Context window (tokens)" }
                                                    div { class: "control",
                                                        input {
                                                            class: "input",
                                                            r#type: "number",
                                                            min: "1",
                                                            placeholder: "engine default",
                                                            value: "{form.context_length}",
                                                            oninput: {
                                                                let eid = eid.clone();
                                                                let fallback = fallback_config.clone();
                                                                let pf = params_form;
                                                                move |evt: FormEvent| {
                                                                    let value = evt.value();
                                                                    params_set(pf, &eid, &fallback, |f| f.context_length = value);
                                                                }
                                                            },
                                                        }
                                                    }
                                                }
                                            }
                                            if engine.supports_joshua_parameters() {
                                                div { class: "column is-half",
                                                    label { class: "label is-size-7", "Compute device (--device)" }
                                                    div { class: "control",
                                                        dioxus_bulma::prelude::Select {
                                                            size: dioxus_bulma::prelude::BulmaSize::Small,
                                                            value: "{form.device}",
                                                            onchange: {
                                                                let eid = eid.clone();
                                                                let fallback = fallback_config.clone();
                                                                let pf = params_form;
                                                                move |evt: FormEvent| {
                                                                    let value = evt.value();
                                                                    params_set(pf, &eid, &fallback, |f| f.device = value);
                                                                }
                                                            },
                                                            option { value: "", "engine default (auto)" }
                                                            option { value: "auto", selected: form.device == "auto", "auto" }
                                                            option { value: "cpu", selected: form.device == "cpu", "cpu" }
                                                            option { value: "metal", selected: form.device == "metal", "metal" }
                                                            option { value: "cuda", selected: form.device == "cuda", "cuda" }
                                                        }
                                                    }
                                                }
                                                div { class: "column is-half",
                                                    label { class: "label is-size-7", "Huge pages (--huge-pages)" }
                                                    div { class: "control",
                                                        dioxus_bulma::prelude::Select {
                                                            size: dioxus_bulma::prelude::BulmaSize::Small,
                                                            value: "{form.huge_pages}",
                                                            onchange: {
                                                                let eid = eid.clone();
                                                                let fallback = fallback_config.clone();
                                                                let pf = params_form;
                                                                move |evt: FormEvent| {
                                                                    let value = evt.value();
                                                                    params_set(pf, &eid, &fallback, |f| f.huge_pages = value);
                                                                }
                                                            },
                                                            option { value: "", "off (default)" }
                                                            option { value: "transparent", selected: form.huge_pages == "transparent", "transparent" }
                                                            option { value: "2mb", selected: form.huge_pages == "2mb", "2mb" }
                                                            option { value: "1gb", selected: form.huge_pages == "1gb", "1gb" }
                                                            option { value: "huge", selected: form.huge_pages == "huge", "huge" }
                                                        }
                                                    }
                                                }
                                                div { class: "column is-half",
                                                    label { class: "label is-size-7", "Max output tokens (--max-output-tokens)" }
                                                    div { class: "control",
                                                        input {
                                                            class: "input",
                                                            r#type: "number",
                                                            min: "1",
                                                            placeholder: "4096 (joshua default)",
                                                            value: "{form.max_output_tokens}",
                                                            oninput: {
                                                                let eid = eid.clone();
                                                                let fallback = fallback_config.clone();
                                                                let pf = params_form;
                                                                move |evt: FormEvent| {
                                                                    let value = evt.value();
                                                                    params_set(pf, &eid, &fallback, |f| f.max_output_tokens = value);
                                                                }
                                                            },
                                                        }
                                                    }
                                                }
                                                div { class: "column is-half",
                                                    label { class: "label is-size-7", "Max concurrent requests (--max-concurrency)" }
                                                    div { class: "control",
                                                        input {
                                                            class: "input",
                                                            r#type: "number",
                                                            min: "1",
                                                            placeholder: "CPU count (joshua default)",
                                                            value: "{form.max_concurrency}",
                                                            oninput: {
                                                                let eid = eid.clone();
                                                                let fallback = fallback_config.clone();
                                                                let pf = params_form;
                                                                move |evt: FormEvent| {
                                                                    let value = evt.value();
                                                                    params_set(pf, &eid, &fallback, |f| f.max_concurrency = value);
                                                                }
                                                            },
                                                        }
                                                    }
                                                }
                                                div { class: "column is-full",
                                                    label { class: "checkbox is-size-7",
                                                        input {
                                                            r#type: "checkbox",
                                                            checked: form.mmap,
                                                            onchange: {
                                                                let eid = eid.clone();
                                                                let fallback = fallback_config.clone();
                                                                let pf = params_form;
                                                                move |evt: FormEvent| {
                                                                    let checked = evt.checked();
                                                                    params_set(pf, &eid, &fallback, |f| f.mmap = checked);
                                                                }
                                                            },
                                                        }
                                                        " Require memory-mappable model (--mmap)"
                                                    }
                                                    br {}
                                                    label { class: "checkbox is-size-7",
                                                        input {
                                                            r#type: "checkbox",
                                                            checked: form.lazy_weights,
                                                            onchange: {
                                                                let eid = eid.clone();
                                                                let fallback = fallback_config.clone();
                                                                let pf = params_form;
                                                                move |evt: FormEvent| {
                                                                    let checked = evt.checked();
                                                                    params_set(pf, &eid, &fallback, |f| f.lazy_weights = checked);
                                                                }
                                                            },
                                                        }
                                                        " Optimise for a model far larger than RAM (--lazy-weights)"
                                                    }
                                                }
                                            }
                                            if engine.supports_default_model() {
                                                div { class: "column is-half",
                                                    label { class: "label is-size-7", "Default model (startup)" }
                                                    div { class: "control",
                                                        dioxus_bulma::prelude::Select {
                                                            size: dioxus_bulma::prelude::BulmaSize::Small,
                                                            value: "{form.default_model}",
                                                            onchange: {
                                                                let eid = eid.clone();
                                                                let fallback = fallback_config.clone();
                                                                let pf = params_form;
                                                                move |evt: FormEvent| {
                                                                    let value = evt.value();
                                                                    params_set(pf, &eid, &fallback, |f| f.default_model = value);
                                                                }
                                                            },
                                                            option {
                                                                value: "",
                                                                selected: form.default_model.is_empty(),
                                                                if model_names.is_empty() {
                                                                    "(no local models — refresh)"
                                                                } else {
                                                                    "— none —"
                                                                }
                                                            }
                                                            for mname in model_names.iter() {
                                                                option {
                                                                    value: "{mname}",
                                                                    selected: form.default_model == *mname,
                                                                    "{mname}"
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            if engine.can("start") {
                                                div { class: "column is-half",
                                                    label { class: "checkbox is-size-7",
                                                        input {
                                                            r#type: "checkbox",
                                                            checked: form.auto_start,
                                                            onchange: {
                                                                let eid = eid.clone();
                                                                let fallback = fallback_config.clone();
                                                                let pf = params_form;
                                                                move |evt: FormEvent| {
                                                                    let checked = evt.checked();
                                                                    params_set(pf, &eid, &fallback, |f| f.auto_start = checked);
                                                                }
                                                            },
                                                        }
                                                        " Auto-start with the gateway"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // ── Live install output for this engine ──────────
                        if let Some(output) = data.install_output.get(&engine.id) {
                            div {
                                class: if output.done && output.ok { "notification is-success is-light" }
                                    else if output.done { "notification is-danger is-light" }
                                    else { "notification is-info is-light" },
                                p { strong { "Install — {output.status_line()}" } }
                                pre {
                                    style: "max-height: 12rem; overflow-y: auto; background: transparent; padding: 0.5rem 0;",
                                    for line in output.tail(rustyclaw_view::InstallOutputData::MAX_LINES).iter() {
                                        "{line}\n"
                                    }
                                }
                            }
                        }

                        // ── Models for the active engine ─────────────────
                        {
                            let selected = engine.id.clone();
                            let is_selected = data.selected_engine.as_deref() == Some(engine.id.as_str());
                            rsx! {
                                if is_selected {
                                    div { class: "box",
                                        h5 { class: "title is-5", "Models" }
                                        if data.models.is_empty() {
                                            p { class: "has-text-grey", "(no models)" }
                                        }
                                        table { class: "table is-fullwidth is-hoverable",
                                            thead {
                                                tr {
                                                    th { "Name" }
                                                    th { "Size" }
                                                    th { "Quant" }
                                                    th { "Status" }
                                                    th { "Actions" }
                                                }
                                            }
                                            tbody {
                                                for model in data.models.iter() {
                                                    tr {
                                                        td { "{model.name}" }
                                                        td { "{model.size_display()}" }
                                                        td { "{model.quantization.as_deref().unwrap_or(\"-\")}" }
                                                        td {
                                                            span {
                                                                class: if model.loaded { "tag is-success" } else { "tag is-light" },
                                                                "{model.load_badge()}"
                                                            }
                                                            if let Some(warning) = model.fit_warning() {
                                                                span { class: "tag is-warning ml-1", "{warning}" }
                                                            }
                                                        }
                                                        td {
                                                            {
                                                                let eid = selected.clone();
                                                                let mname = model.name.clone();
                                                                let loaded = model.loaded;
                                                                let engine_caps = data.engine(&selected).cloned();
                                                                let can_load = engine_caps.as_ref().is_some_and(|e| e.can("load"));
                                                                let can_unload = engine_caps.as_ref().is_some_and(|e| e.can("unload"));
                                                                let can_remove = engine_caps.as_ref().is_some_and(|e| e.can("remove"));
                                                                // In-flight feedback: the clicked model's button turns into
                                                                // "Loading…" until the gateway answers.
                                                                let pending_here = props
                                                                    .action_pending
                                                                    .as_ref()
                                                                    .is_some_and(|(pe, pm)| pe == &eid && pm == &mname);
                                                                rsx! {
                                                                    dioxus_bulma::prelude::Buttons {
                                                                        {
                                                                            let eid2 = eid.clone();
                                                                            let mname2 = mname.clone();
                                                                            rsx! {
                                                                                dioxus_bulma::prelude::Button {
                                                                                    color: BulmaColor::Primary,
                                                                                    onclick: move |_| props.on_use_model.call((eid2.clone(), mname2.clone())),
                                                                                    "Use"
                                                                                }
                                                                            }
                                                                        }
                                                                        if !loaded && can_load {
                                                                            {
                                                                                let eid2 = eid.clone();
                                                                                let mname2 = mname.clone();
                                                                                rsx! {
                                                                                    dioxus_bulma::prelude::Button {
                                                                                        color: BulmaColor::Info,
                                                                                        disabled: pending_here,
                                                                                        onclick: move |_| props.on_model_action.call((eid2.clone(), mname2.clone(), ModelActionKind::Load)),
                                                                                        if pending_here { "Loading…" } else { "Load" }
                                                                                    }
                                                                                }
                                                                            }
                                                                        }
                                                                        if loaded && can_unload {
                                                                            {
                                                                                let eid2 = eid.clone();
                                                                                let mname2 = mname.clone();
                                                                                rsx! {
                                                                                    dioxus_bulma::prelude::Button {
                                                                                        color: BulmaColor::Warning,
                                                                                        disabled: pending_here,
                                                                                        onclick: move |_| props.on_model_action.call((eid2.clone(), mname2.clone(), ModelActionKind::Unload)),
                                                                                        if pending_here { "Loading…" } else { "Unload" }
                                                                                    }
                                                                                }
                                                                            }
                                                                        }
                                                                        if can_remove {
                                                                            {
                                                                                let eid2 = eid.clone();
                                                                                let mname2 = mname.clone();
                                                                                rsx! {
                                                                                    dioxus_bulma::prelude::Button {
                                                                                        color: BulmaColor::Danger,
                                                                                        outlined: true,
                                                                                        disabled: pending_here,
                                                                                        onclick: move |_| props.on_model_action.call((eid2.clone(), mname2.clone(), ModelActionKind::Remove)),
                                                                                        "Remove"
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
                                            }
                                        }

                                        // Pull a new model by name (engines that support it).
                                        if data.engine(&selected).is_some_and(|e| e.can("pull")) {
                                            div { class: "field has-addons mt-3",
                                                div { class: "control is-expanded",
                                                    input {
                                                        class: "input",
                                                        placeholder: "Model to pull (e.g. llama3.1:8b)",
                                                        value: "{pull_input}",
                                                        oninput: move |evt| pull_input.set(evt.value()),
                                                    }
                                                }
                                                div { class: "control",
                                                    {
                                                        let eid = selected.clone();
                                                        let pulling = data.pull_progress.is_some();
                                                        rsx! {
                                                            dioxus_bulma::prelude::Button {
                                                                color: BulmaColor::Info,
                                                                disabled: pull_input.read().trim().is_empty() || pulling,
                                                                onclick: move |_| {
                                                                    let model = pull_input.read().trim().to_string();
                                                                    if !model.is_empty() {
                                                                        props.on_pull.call((eid.clone(), model));
                                                                        pull_input.set(String::new());
                                                                    }
                                                                },
                                                                "Pull"
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

                        // Pull progress (shown on its engine's tab).
                        if let Some(ref progress) = data.pull_progress {
                            if progress.engine == engine.id {
                                div { class: "notification is-info is-light mt-3",
                                    p { strong { "Pulling: " } "{progress.model}" }
                                    progress {
                                        class: "progress is-info",
                                        value: "{progress.pct()}",
                                        max: "100",
                                    }
                                    p { class: "is-size-7", "{progress.display()}" }
                                }
                            }
                        }
                    }
                }
            }
            if props.data.is_none() {
                div { class: "has-text-centered py-6",
                    p { class: "has-text-grey", "Loading engine data..." }
                }
            }
        }
    }
}

#[allow(dead_code)]
fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else if bytes >= 1_000_000 {
        format!("{:.0} MB", bytes as f64 / 1e6)
    } else {
        format!("{:.0} KB", bytes as f64 / 1e3)
    }
}
