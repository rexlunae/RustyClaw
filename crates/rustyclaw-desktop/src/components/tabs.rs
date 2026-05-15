//! Thread tab bar — replaces the old thread list in the sidebar.
//!
//! Renders horizontally above the message area.  Each tab shows
//! the session label, message count, and a close button.

use dioxus::prelude::*;
use rustyclaw_view::TabBarData;

#[derive(Props, Clone, PartialEq)]
pub struct TabBarProps {
    pub data: TabBarData,
    pub on_switch: EventHandler<u64>,
    pub on_new: EventHandler<()>,
    pub on_close: EventHandler<u64>,
}

/// Pre-computed row — avoids lifetime issues with closures inside RSX for loops.
struct TabRow {
    key: u64,
    is_active: bool,
    label: String,
    count_str: String,
    closeable: bool,
}

#[component]
pub fn TabBar(props: TabBarProps) -> Element {
    let total = props.data.tabs.len();
    let fg_id = props.data.foreground_id;

    // Pre-compute rows outside the rsx! macro
    let rows: Vec<TabRow> = props.data.tabs.iter().map(|tab| {
        TabRow {
            key: tab.id,
            is_active: tab.id == fg_id,
            label: tab.truncated_label(24).into_owned(),
            count_str: format!("({})", tab.message_count),
            closeable: tab.closeable(total),
        }
    }).collect();

    // Extract handlers to owned scope so closures in RSX don't fight over props
    let on_switch = props.on_switch;
    let on_close = props.on_close;
    let on_new = props.on_new;

    rsx! {
        div { class: "tab-bar",
            div { class: "tab-bar-tabs",
                for row in &rows {
                    div {
                        class: if row.is_active {
                            "tab-bar-tab is-active"
                        } else {
                            "tab-bar-tab"
                        },
                        key: "{row.key}",
                        onclick: {
                            let id = row.key;
                            move |_| on_switch.call(id)
                        },
                        span { class: "tab-bar-label", "{row.label}" }
                        span { class: "tab-bar-count", "{row.count_str}" }
                        if row.closeable {
                            button {
                                class: "tab-bar-close",
                                title: "Close session",
                                onclick: {
                                    let id = row.key;
                                    move |evt: MouseEvent| {
                                        evt.stop_propagation();
                                        on_close.call(id);
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
                onclick: move |_| on_new.call(()),
                "+"
            }
        }
    }
}
