use dioxus::prelude::*;
use dioxus_bulma::prelude::*;
use crate::state::AppState;

#[component]
pub fn SettingsDialog(
    visible: Signal<bool>,
    state: Signal<AppState>,
    on_close: EventHandler<()>,
) -> Element {
    let mut gateway_url = use_signal(|| state.read().gateway_url.clone());

    let save_settings = move |_| {
        state.write().gateway_url = gateway_url.read().clone();
        on_close.call(());
    };

    rsx! {
        div { 
            class: "modal",
            style: if *visible.read() { "display: block" } else { "display: none" },
            div { class: "modal-background", onclick: move |_| on_close.call(()) }
            div { 
                class: "modal-card",
                div { 
                    class: "modal-card-head",
                    p { class: "modal-card-title", "Settings" }
                }
                div { 
                    class: "modal-card-body",
                    div { 
                        class: "field",
                        div { 
                            class: "field-label",
                            label { class: "label", "Gateway URL" }
                        }
                        div { 
                            class: "field-body",
                            Input { 
                                value: gateway_url.read().clone(),
                                oninput: move |e| gateway_url.set(e),
                                placeholder: "ws://127.0.0.1:9001",
                            }
                        }
                    }
                    div { 
                        class: "field",
                        div { 
                            class: "field-label",
                            label { class: "label", "Active Model" }
                        }
                        div { 
                            class: "field-body",
                            p { 
                                class: "is-size-7", 
                                "{state.read().model.as_deref().unwrap_or(\"None connected\")}" 
                            }
                        }
                    }
                    div { 
                        class: "field",
                        div { 
                            class: "field-label",
                            label { class: "label", "Provider" }
                        }
                        div { 
                            class: "field-body",
                            p { 
                                class: "is-size-7", 
                                "{state.read().provider.as_deref().unwrap_or(\"None connected\")}" 
                            }
                        }
                    }
                }
                div { 
                    class: "modal-card-foot",
                    button { 
                        class: "button", 
                        onclick: move |_| on_close.call(()), 
                        "Cancel" 
                    }
                    button { 
                        class: "button is-primary", 
                        onclick: save_settings, 
                        "Save Changes" 
                    }
                }
            }
        }
    }
}
