//! Thread tab bar — replaces the old thread list in the sidebar.
//!
//! Renders horizontally above the message area.  Each tab shows
//! the session label, message count, and a close button.  Active
//! tab is highlighted.

use dioxus::prelude::*;
use rustyclaw_view::TabBarData;

#[derive(Props, Clone, PartialEq)]
pub struct TabBarProps {
    pub data: TabBarData,
    pub on_switch: EventHandler<u64>,
    pub on_new: EventHandler<()>,
    pub on_close: EventHandler<u64>,
}

/// Pre-computed tab fields for RSX rendering (avoids `let` inside loops).
struct TabRow {
    key: u64,
    cls: String,
    label: String,
    count_str: String,
    closeable: bool,
}

#[component]
pub fn TabBar(props: TabBarProps) -> Element {
    let d = &props.data;
    let total = d.tabs.len();

    // Pre-compute tab rows so closures own their data.
    let rows: Vec<TabRow> = d.tabs.iter().map(|tab| {
        TabRow {
            key: tab.id,
            cls: if tab.id == d.foreground_id {
                "tab-bar-tab is-active".to_string()
            } else {
                "tab-bar-tab".to_string()
            },
            label: tab.truncated_label(24).into_owned(),
            count_str: format!("({})", tab.message_count),
            closeable: tab.closeable(total),
        }
    }).collect();

    rsx! {
        div { class: "tab-bar",
            div { class: "tab-bar-tabs",
                for row in &rows {
                    div {
                        class: "{row.cls}",
                        key: "{row.key}",
                        onclick: {
                            let id = row.key;
                            move |_| props.on_switch.call(id)
                        },

                        span { class: "tab-bar-label", "{row.label}" }
                        span { class: "tab-bar-count", "{row.count_str}" }

                        if row.closeable {
                            button {
                                class: "tab-bar-close",
                                title: "Close session",
                                onclick: {
                                    let id = row.key;
                                    move |evt| {
                                        evt.stop_propagation();
                                        props.on_close.call(id);
                                    }
                                },
                                "\u{2715}"
                            }
                        }
                    }
                }
            }

            button {
                class: "tab-bar-new",
                title: "New session",
                onclick: move |_| props.on_new.call(()),
                "+"
            }
        }
    }
}
