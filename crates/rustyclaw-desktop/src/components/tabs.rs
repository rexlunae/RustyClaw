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

#[component]
pub fn TabBar(props: TabBarProps) -> Element {
    let total = props.data.tabs.len();
    let fg_id = props.data.foreground_id;

    // Extract handlers to owned scope so closures don't fight over props
    let on_switch = props.on_switch;
    let on_close = props.on_close;
    let on_new = props.on_new;

    // Build tab elements OUTSIDE the rsx macro in plain Rust,
    // avoiding the for-loop closure-capture bug in Dioxus 0.7 RSX.
    let mut tab_nodes: Vec<VNode> = Vec::with_capacity(total);

    for tab in &props.data.tabs {
        let cls = if tab.id == fg_id {
            "tab-bar-tab is-active"
        } else {
            "tab-bar-tab"
        };
        let label = tab.truncated_label(24).into_owned();
        let count_str = format!("({})", tab.message_count);
        let show_close = tab.closeable(total);
        let tab_id = tab.id;

        // Each rsx! call gets its own immutable copy of tab_id (u64) and
        // a clone of the event handler — no shared mutable reference.
        let node = rsx! {
            div {
                class: "{cls}",
                key: "{tab_id}",
                onclick: move |_| on_switch.call(tab_id),
                span { class: "tab-bar-label", "{label}" }
                span { class: "tab-bar-count", "{count_str}" }
                if show_close {
                    button {
                        class: "tab-bar-close",
                        title: "Close session",
                        onclick: move |evt: MouseEvent| {
                            evt.stop_propagation();
                            on_close.call(tab_id);
                        },
                        "\u{2715}"
                    }
                }
            }
        };
        // In Dioxus 0.7, rsx!() returns Result<VNode, RenderError>.
        // We unwrap here because the template is static — no dynamic
        // runtime rendering errors are possible.
        tab_nodes.push(node.unwrap());
    }

    // Show "No sessions" placeholder when there are no threads at all.
    if tab_nodes.is_empty() {
        tab_nodes.push(rsx! {
            div { class: "tab-bar-empty",
                span { "No sessions" }
            }
        }.unwrap());
    }

    rsx! {
        div { class: "tab-bar",
            div { class: "tab-bar-tabs",
                {tab_nodes.into_iter()}
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
