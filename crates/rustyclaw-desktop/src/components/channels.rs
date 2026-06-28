//! Channels panel — messenger channel status dialog.

use dioxus::prelude::*;
use dioxus_bulma::prelude::BulmaColor;

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct ChannelsDialogProps {
    pub visible: bool,
    pub data: Option<rustyclaw_view::ChannelsPanelData>,
    pub on_close: EventHandler<()>,
}

#[component]
pub fn ChannelsDialog(props: ChannelsDialogProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    rsx! {
        RcModal {
            active: true,
            title: "Channels",
            width: 650,
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
                if data.channels.is_empty() {
                    p { class: "has-text-grey", "No channels configured." }
                } else {
                    div { class: "mb-3",
                        span { class: "tag is-info is-light",
                            "{data.online_count()} online / {data.paired_count()} paired / {data.total_count()} total"
                        }
                    }

                    table { class: "table is-narrow is-fullwidth is-hoverable",
                        thead {
                            tr {
                                th { "Channel" }
                                th { "Type" }
                                th { "Status" }
                                th { "Last Message" }
                            }
                        }
                        tbody {
                            for ch in data.channels.iter() {
                                tr { key: "{ch.name}",
                                    td { strong { "{ch.channel_icon()} {ch.name}" } }
                                    td { "{ch.channel_type}" }
                                    td {
                                        span { class: "tag",
                                            "{ch.status_label()}"
                                        }
                                    }
                                    td {
                                        if let Some(ref last) = ch.last_message {
                                            "{last}"
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
                p { class: "has-text-grey", "Channel system not initialised." }
            }
        }
    }
}
