//! Engines panel — local engine and model management dialog.
//!
//! Laid out as one tab per detected engine: the tab strip switches the
//! active engine, and the body shows that engine's status, actions, models,
//! live install output, and any pull progress.

use dioxus::prelude::*;
use dioxus_bulma::prelude::BulmaColor;
use rustyclaw_core::gateway::{EngineActionKind, ModelActionKind};

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct EnginesDialogProps {
    pub visible: bool,
    pub data: Option<rustyclaw_view::EnginesPanelData>,
    pub on_close: EventHandler<()>,
    pub on_engine_action: EventHandler<(String, EngineActionKind)>,
    pub on_model_action: EventHandler<(String, String, ModelActionKind)>,
    pub on_pull: EventHandler<(String, String)>,
    /// Select an engine to browse its models (sends a model-list request).
    /// Also used by the tab strip to switch the active engine.
    pub on_select_engine: EventHandler<String>,
    /// Switch the active chat provider/model to this local (engine, model).
    pub on_use_model: EventHandler<(String, String)>,
    /// Re-fetch the engine list (and selected engine's models).
    pub on_refresh: EventHandler<()>,
}

#[component]
pub fn EnginesDialog(props: EnginesDialogProps) -> Element {
    let mut pull_input = use_signal(String::new);

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
                                                                                        onclick: move |_| props.on_model_action.call((eid2.clone(), mname2.clone(), ModelActionKind::Load)),
                                                                                        "Load"
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
                                                                                        onclick: move |_| props.on_model_action.call((eid2.clone(), mname2.clone(), ModelActionKind::Unload)),
                                                                                        "Unload"
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
