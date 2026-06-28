//! Analytics panel — token/usage dashboard dialog.

use dioxus::prelude::*;
use dioxus_bulma::prelude::BulmaColor;
use rustyclaw_view::analytics::UsageTotalsData;

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct AnalyticsDialogProps {
    pub visible: bool,
    pub data: Option<rustyclaw_view::AnalyticsPanelData>,
    pub on_close: EventHandler<()>,
}

#[component]
pub fn AnalyticsDialog(props: AnalyticsDialogProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    rsx! {
        RcModal {
            active: true,
            title: "Usage Analytics",
            width: 750,
            onclose: move |_| props.on_close.call(()),
            footer: rsx! {
                dioxus_bulma::prelude::Buttons {
                    dioxus_bulma::prelude::Button {
                        color: BulmaColor::Primary,
                        onclick: move |_| props.on_close.call(()),
                        "Close"
                    }
                }
            },

            if let Some(ref data) = props.data {
                // Summary tiles
                div { class: "columns is-multiline mb-4",
                    div { class: "column is-3",
                        div { class: "box has-text-centered",
                            p { class: "heading", "Requests" }
                            p { class: "title is-5", "{data.totals.total_requests}" }
                        }
                    }
                    div { class: "column is-3",
                        div { class: "box has-text-centered",
                            p { class: "heading", "Input Tokens" }
                            p { class: "title is-5", "{UsageTotalsData::tokens_display(data.totals.total_input_tokens)}" }
                        }
                    }
                    div { class: "column is-3",
                        div { class: "box has-text-centered",
                            p { class: "heading", "Output Tokens" }
                            p { class: "title is-5", "{UsageTotalsData::tokens_display(data.totals.total_output_tokens)}" }
                        }
                    }
                    div { class: "column is-3",
                        div { class: "box has-text-centered",
                            p { class: "heading", "Avg Latency" }
                            p { class: "title is-5", "{data.totals.avg_latency_ms()}ms" }
                        }
                    }
                }

                // Per-model table
                if !data.per_model.is_empty() {
                    h5 { class: "subtitle is-6 mt-4", "By Model" }
                    table { class: "table is-narrow is-fullwidth is-hoverable",
                        thead {
                            tr {
                                th { "Provider" }
                                th { "Model" }
                                th { "Requests" }
                                th { "Tokens" }
                                th { "Avg Latency" }
                            }
                        }
                        tbody {
                            for m in data.per_model.iter() {
                                tr {
                                    td { "{m.provider}" }
                                    td { strong { "{m.model}" } }
                                    td { "{m.requests}" }
                                    td { "{UsageTotalsData::tokens_display(m.total_tokens())}" }
                                    td { "{m.avg_latency_ms}ms" }
                                }
                            }
                        }
                    }
                }

                // Per-session table
                if !data.per_session.is_empty() {
                    h5 { class: "subtitle is-6 mt-4", "By Session" }
                    table { class: "table is-narrow is-fullwidth is-hoverable",
                        thead {
                            tr {
                                th { "Session" }
                                th { "Requests" }
                                th { "Input" }
                                th { "Output" }
                            }
                        }
                        tbody {
                            for s in data.per_session.iter() {
                                tr {
                                    td { "{s.display_label()}" }
                                    td { "{s.requests}" }
                                    td { "{UsageTotalsData::tokens_display(s.input_tokens)}" }
                                    td { "{UsageTotalsData::tokens_display(s.output_tokens)}" }
                                }
                            }
                        }
                    }
                }
            } else {
                p { class: "has-text-grey", "Analytics not available." }
            }
        }
    }
}
