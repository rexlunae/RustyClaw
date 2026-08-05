//! Downloads panel — file transfers the agent started with `web_fetch`.
//!
//! Deliberately not in the transcript. A transfer outlives the turn that
//! started it, it changes many times a second while it runs, and the user
//! wants to see where a file went without scrolling back through a
//! conversation. The panel is the live view; the agent gets one message when
//! a transfer ends, and that one does belong in the transcript.

use dioxus::prelude::*;
use dioxus_bulma::prelude::BulmaColor;

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct DownloadsDialogProps {
    pub visible: bool,
    pub downloads: rustyclaw_view::DownloadsData,
    pub on_close: EventHandler<()>,
    pub on_cancel: EventHandler<String>,
    pub on_clear_finished: EventHandler<()>,
}

#[component]
pub fn DownloadsDialog(props: DownloadsDialogProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    let data = props.downloads;
    let has_finished = data.has_finished();

    rsx! {
        RcModal {
            active: true,
            title: "Downloads",
            width: 720,
            onclose: move |_| props.on_close.call(()),
            footer: rsx! {
                dioxus_bulma::prelude::Buttons {
                    dioxus_bulma::prelude::Button {
                        // Disabled rather than hidden: a button that appears
                        // and disappears as transfers finish is a moving
                        // target, and the label is what says what it does.
                        disabled: !has_finished,
                        onclick: move |_| props.on_clear_finished.call(()),
                        "Clear finished"
                    }
                    dioxus_bulma::prelude::Button {
                        color: BulmaColor::Primary,
                        onclick: move |_| props.on_close.call(()),
                        "Close"
                    }
                }
            },

            if data.is_empty() {
                p { class: "has-text-grey",
                    "No downloads yet. The agent starts one by fetching a URL to a file."
                }
            } else {
                for d in data.downloads.iter() {
                    div { key: "{d.id}", class: "box p-3 mb-2",
                        div { class: "is-flex is-justify-content-space-between is-align-items-baseline",
                            div { class: "is-flex-grow-1",
                                strong { "{d.file_name()}" }
                                span { class: "tag {d.status_class()} is-light ml-2",
                                    "{d.status_label()}"
                                }
                            }
                            // Only while it is running: the gateway refuses a
                            // cancel for a transfer that has ended, so showing
                            // the button afterwards would offer an action that
                            // does nothing.
                            if d.is_running() {
                                {
                                    let id = d.id.clone();
                                    rsx! {
                                        dioxus_bulma::prelude::Button {
                                            color: BulmaColor::Danger,
                                            onclick: move |_| props.on_cancel.call(id.clone()),
                                            "Cancel"
                                        }
                                    }
                                }
                            }
                        }

                        // The destination is the reason the user opened this
                        // panel: the agent chose where the file went.
                        p { class: "is-size-7 has-text-grey mt-1", title: "{d.url}",
                            "{d.dest}"
                        }

                        if d.is_running() {
                            match d.percent() {
                                // A declared length gives a real bar.
                                Some(pct) => rsx! {
                                    progress {
                                        class: "progress is-info is-small mt-2",
                                        value: "{pct}",
                                        max: "100",
                                        "{pct}%"
                                    }
                                },
                                // A chunked response declares none. An
                                // indeterminate bar says "working" without
                                // claiming a fraction that does not exist.
                                None => rsx! {
                                    progress { class: "progress is-info is-small mt-2" }
                                },
                            }
                        }

                        p { class: "is-size-7 has-text-grey",
                            "{d.size_display()}"
                            if let Some(pct) = d.percent() {
                                if d.is_running() {
                                    " · {pct}%"
                                }
                            }
                            if let Some(took) = d.duration_display() {
                                " · {took}"
                            }
                        }

                        // Shown in full rather than as "failed": whether to
                        // retry, use another URL, or give up depends entirely
                        // on what went wrong.
                        if let Some(error) = d.error.as_ref() {
                            p { class: "is-size-7 has-text-danger mt-1", "{error}" }
                        }
                    }
                }
            }
        }
    }
}
