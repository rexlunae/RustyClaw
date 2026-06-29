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

    let header = format!(
        "{} · {} {}",
        props.data.summary(),
        status_icon,
        status_label
    );

    // Short inline peek shown when collapsed so the row is one line but still
    // hints at what ran / what came back.
    let peek = if collapsed {
        let src = props
            .data
            .result_preview(80, 1)
            .unwrap_or_else(|| props.data.arguments_preview(80, 1));
        let one = src.replace('\n', " ");
        if one.trim().is_empty() {
            String::new()
        } else {
            format!("  {one}")
        }
    } else {
        String::new()
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
                content: format!("{header}{peek}"),
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
