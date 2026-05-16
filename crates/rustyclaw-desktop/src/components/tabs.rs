//! Thread tab bar — replaces the old thread list in the sidebar.
//!
//! Renders horizontally above the message area.  Each tab shows
//! the session label, message count, and a close button.

use dioxus::prelude::*;
use rustyclaw_view::TabBarData;

#[derive(Props, Clone)]
pub struct TabBarProps {
    pub data: TabBarData,
    pub on_switch: EventHandler<u64>,
    pub on_new: EventHandler<()>,
    pub on_close: EventHandler<u64>,
}

/// Always re-render — the data is derived from state on every render.
impl PartialEq for TabBarProps {
    fn eq(&self, _other: &Self) -> bool {
        false
    }
}

/// Thread tab bar component.
///
/// Rendered as an always-render component so that fresh thread data
/// from every parent re-render always reaches the DOM.
#[component]
pub fn TabBar(props: TabBarProps) -> Element {
    let fg_id = props.data.foreground_id;
    let total = props.data.tabs.len();
    let has_tabs = total > 0;

    let on_switch = props.on_switch;
    let on_close = props.on_close;
    let on_new = props.on_new;

    rsx! {
        div { class: "tab-bar",
            div { class: "tab-bar-tabs",
                {
                    props.data.tabs.iter().map(|tab| {
                        let cls = if tab.id == fg_id {
                            "tab-bar-tab is-active"
                        } else {
                            "tab-bar-tab"
                        };
                        let label = tab.truncated_label(24).into_owned();
                        let count_str = format!("({})", tab.message_count);
                        let show_close = tab.closeable(total);
                        let tab_id = tab.id;

                        rsx! {
                            div {
                                class: "{cls}",
                                key: "{tab_id}",
                                onclick: move |_| on_switch.call(tab_id),
                                span { class: "tab-bar-label", "{label}" }
                                span { class: "tab-bar-count", "{count_str}" }
                                if show_close {
                                    button {
                                        class: "tab-bar-close",
                                        title: "Delete thread",
                                        onclick: move |evt: MouseEvent| {
                                            evt.stop_propagation();
                                            on_close.call(tab_id);
                                        },
                                        "\u{2715}"
                                    }
                                }
                            }
                        }
                    })
                }
                if !has_tabs {
                    div { class: "tab-bar-empty",
                        span { "No sessions" }
                    }
                }
            }

            button {
                class: "tab-bar-new",
                title: "New session (Ctrl+Shift+E)",
                onclick: move |_| on_new.call(()),
                "+"
            }
        }
    }
}
