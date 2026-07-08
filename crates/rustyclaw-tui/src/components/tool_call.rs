use crate::theme;
use iocraft::prelude::*;

/// How many trailing lines of a running tool's live output to show.
const LIVE_TAIL_LINES: usize = 6;

#[derive(Default, Props)]
pub struct ToolCallPanelProps {
    pub data: rustyclaw_view::ToolCallData,
}

#[component]
pub fn ToolCallPanel(props: &ToolCallPanelProps) -> impl Into<AnyElement<'static>> {
    let (_, status_label, status_icon) = props.data.status_label();
    let color = if props.data.is_error {
        theme::ERROR
    } else {
        theme::INFO
    };

    // Presentation states:
    //  - Running → always "open": header plus the live output tail,
    //    updating in place as the process writes (the collapsed flag is
    //    ignored — a running command is exactly what the user wants to
    //    watch).
    //  - Done, collapsed (default) → a single dim line: what ran, the
    //    outcome, how long it took, and a one-line gist of the result.
    //  - Done, expanded (Ctrl+E) → header plus truncated args + result.
    let running = props.data.is_running();
    let collapsed = props.data.collapsed && !running;

    let action = props.data.compact_action();
    let duration = props
        .data
        .duration_label()
        .map(|d| format!(" {d}"))
        .unwrap_or_default();
    let header = if running {
        format!("🔧 {action} · {status_icon} {status_label}")
    } else if collapsed {
        let gist = props
            .data
            .result_gist()
            .map(|g| format!(" · {g}"))
            .unwrap_or_default();
        format!("🔧 {action} · {status_icon}{duration}{gist}")
    } else {
        format!(
            "🔧 {} — {action} · {status_icon} {status_label}{duration}",
            props.data.name
        )
    };

    // Live tail while running; args/result detail when expanded after
    // completion.
    let live_tail = if running {
        props.data.live_tail(LIVE_TAIL_LINES)
    } else {
        None
    };
    let args = if collapsed || running {
        String::new()
    } else {
        props.data.arguments_preview(600, 12)
    };
    let result = if collapsed || running {
        None
    } else {
        props.data.result_preview(2000, 40)
    };

    // Live status while the call runs: elapsed time plus CPU/state/memory
    // of the child process, with inline control hints when controllable.
    let live_status = props.data.live_status_line().map(|line| {
        if props.data.is_controllable() {
            let pause_hint = if props.data.is_process_paused() {
                "Ctrl+Z resume"
            } else {
                "Ctrl+Z pause"
            };
            format!("{line} — {pause_hint} · Ctrl+T stop · Ctrl+K kill")
        } else {
            line
        }
    });

    element! {
        View(
            width: 100pct,
            padding_left: 2,
            padding_right: 1,
            flex_direction: FlexDirection::Column,
        ) {
            Text(
                content: header,
                color,
                weight: if collapsed { Weight::Normal } else { Weight::Bold },
            )
            #(if let Some(status) = live_status {
                element! {
                    Text(content: status, color: theme::WARN, wrap: TextWrap::Wrap)
                }.into_any()
            } else {
                element! { View() }.into_any()
            })
            #(if let Some(tail) = live_tail {
                element! {
                    Text(content: tail, color: theme::TEXT_DIM, wrap: TextWrap::Wrap)
                }.into_any()
            } else {
                element! { View() }.into_any()
            })
            #(if !args.is_empty() {
                element! {
                    Text(content: format!("→ {args}"), color: theme::TEXT_DIM, wrap: TextWrap::Wrap)
                }.into_any()
            } else {
                element! { View() }.into_any()
            })
            #(if let Some(result) = result {
                element! {
                    Text(
                        content: format!("↳ {result}"),
                        color: if props.data.is_error { theme::ERROR } else { theme::TEXT },
                        wrap: TextWrap::Wrap,
                    )
                }.into_any()
            } else {
                element! { View() }.into_any()
            })
        }
    }
}
