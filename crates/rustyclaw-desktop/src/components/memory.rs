//! Memory panel — memory browser dialog.

use dioxus::prelude::*;
use dioxus_bulma::prelude::BulmaColor;

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct MemoryDialogProps {
    pub visible: bool,
    pub data: Option<rustyclaw_view::MemoryPanelData>,
    pub on_close: EventHandler<()>,
}

#[component]
pub fn MemoryDialog(props: MemoryDialogProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    rsx! {
        RcModal {
            active: true,
            title: "Memory Browser",
            width: 700,
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
                if data.entries.is_empty() {
                    p { class: "has-text-grey", "No memory entries." }
                } else {
                    div { class: "mb-3",
                        span { class: "tag is-info is-light",
                            "{data.count()} entries"
                        }
                    }

                    table { class: "table is-narrow is-fullwidth is-hoverable",
                        thead {
                            tr {
                                th { "Category" }
                                th { "Content" }
                                th { "Score" }
                                th { "Updated" }
                            }
                        }
                        tbody {
                            for entry in data.entries.iter() {
                                tr { key: "{entry.id}",
                                    td {
                                        span { class: "tag is-light",
                                            "{entry.category_label()}"
                                        }
                                    }
                                    td { "{entry.preview(80)}" }
                                    td {
                                        if let Some(ref score) = entry.score_display() {
                                            span { class: "tag is-primary is-light",
                                                "{score}"
                                            }
                                        } else {
                                            "—"
                                        }
                                    }
                                    td {
                                        if let Some(ref updated) = entry.updated_at {
                                            "{updated}"
                                        } else {
                                            "—"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                p { class: "has-text-grey", "Memory system not initialised." }
            }
        }
    }
}
