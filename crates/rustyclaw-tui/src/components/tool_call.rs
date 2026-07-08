use crate::theme;
use iocraft::prelude::*;

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

    // Collapsed (default) → single dim line: header + status + arg/result peek.
    // Expanded → header line plus truncated args and result beneath.
    let collapsed = props.data.collapsed;

    // Compact but informative: what the call *does* (derived from the
    // arguments), the outcome, how long it took, and a one-line gist of
    // what came back — all on a single dim row when collapsed.
    let action = props.data.compact_action();
    let duration = props
        .data
        .duration_label()
        .map(|d| format!(" {d}"))
        .unwrap_or_default();
    let header = if collapsed {
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

    let args = if collapsed {
        String::new()
    } else {
        props.data.arguments_preview(600, 12)
    };
    let result = if collapsed {
        None
    } else {
        props.data.result_preview(2000, 40)
    };

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
