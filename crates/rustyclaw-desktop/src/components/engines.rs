//! Engines panel — local engine and model management dialog.

use dioxus::prelude::*;
use dioxus_bulma::prelude::BulmaColor;

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct EnginesDialogProps {
    pub visible: bool,
    pub data: Option<rustyclaw_view::EnginesPanelData>,
    pub on_close: EventHandler<()>,
    pub on_engine_action: EventHandler<(String, String)>,
    pub on_model_action: EventHandler<(String, String, String)>,
    pub on_pull: EventHandler<(String, String)>,
    /// Select an engine to browse its models (sends a model-list request).
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
            // Resource header
            if let Some(ref data) = props.data {
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

                // Engine list
                for engine in data.engines.iter() {
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
                                        span { class: "is-size-7 has-text-grey",
                                            "v{ver}"
                                        }
                                    }
                                }
                            }
                            div { class: "level-right",
                                if !engine.installed && engine.can("install") {
                                    div { class: "level-item",
                                        {
                                            let eid = engine.id.clone();
                                            rsx! {
                                                dioxus_bulma::prelude::Button {
                                                    color: BulmaColor::Info,
                                                    onclick: move |_| props.on_engine_action.call((eid.clone(), "install".into())),
                                                    "Install"
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
                                                    onclick: move |_| props.on_engine_action.call((eid.clone(), "start".into())),
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
                                                    onclick: move |_| props.on_engine_action.call((eid.clone(), "stop".into())),
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
                                            let is_selected = data.selected_engine.as_deref() == Some(engine.id.as_str());
                                            rsx! {
                                                dioxus_bulma::prelude::Button {
                                                    color: if is_selected { BulmaColor::Link } else { BulmaColor::Ghost },
                                                    onclick: move |_| props.on_select_engine.call(eid.clone()),
                                                    "Models"
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
                                p { class: "is-size-7 has-text-grey",
                                    "Endpoint: {ep}"
                                }
                            }
                        }
                    }
                }

                // Models for selected engine
                if let Some(ref selected) = data.selected_engine {
                    div { class: "box",
                        h5 { class: "title is-5", "Models ({selected})" }
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
                                                let engine_caps = data.engine(selected).cloned();
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
                                                                        onclick: move |_| props.on_model_action.call((eid2.clone(), mname2.clone(), "load".into())),
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
                                                                        onclick: move |_| props.on_model_action.call((eid2.clone(), mname2.clone(), "unload".into())),
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
                                                                        onclick: move |_| props.on_model_action.call((eid2.clone(), mname2.clone(), "remove".into())),
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
                        if data.engine(selected).is_some_and(|e| e.can("pull")) {
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

                // Pull progress
                if let Some(ref progress) = data.pull_progress {
                    div { class: "notification is-info is-light mt-3",
                        p { strong { "Pulling: " } "{progress.model}" }
                        progress {
                            class: "progress is-info",
                            value: "{progress.pct()}",
                            max: "100",
                        }
                        p { class: "is-size-7",
                            "{progress.display()}"
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
