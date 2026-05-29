use crate::theme;
use iocraft::prelude::*;

#[derive(Default, Props)]
pub struct ToolCallPanelProps {
    pub data: rustyclaw_view::ToolCallData,
}

#[component]
pub fn ToolCallPanel(props: &ToolCallPanelProps) -> impl Into<AnyElement<'static>> {
    let status = if props.data.result.is_some() {
        if props.data.is_error {
            "✕ Error"
        } else {
            "✓ Done"
        }
    } else {
        "⏳ Running"
    };

    let args = props.data.arguments_preview(600, 12);
    let result = props.data.result_preview(2000, 40);

    element! {
        View(
            width: 100pct,
            margin_bottom: 1,
            padding_left: 2,
            padding_right: 1,
            border_style: BorderStyle::Round,
            border_color: if props.data.is_error { theme::ERROR } else { theme::INFO },
            border_edges: Edges::Left,
            flex_direction: FlexDirection::Column,
        ) {
            Text(
                content: format!("{} · {}", props.data.summary(), status),
                color: if props.data.is_error { theme::ERROR } else { theme::ACCENT },
                weight: Weight::Bold,
            )
            Text(content: args, color: theme::TEXT_DIM, wrap: TextWrap::Wrap)
            #(if let Some(result) = result {
                element! {
                    Text(
                        content: result,
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