// ── Tool approval dialog — ask user to approve/deny a tool call ─────────────

use crate::theme;
use iocraft::prelude::*;
use rustyclaw_view::ToolApprovalData;

#[derive(Default, Props)]
pub struct ToolApprovalDialogProps {
    /// Shared dialog data from `rustyclaw-view`.
    pub data: ToolApprovalData,
}

#[component]
pub fn ToolApprovalDialog(props: &ToolApprovalDialogProps) -> impl Into<AnyElement<'static>> {
    let allow_color = if props.data.selected_allow {
        theme::SUCCESS
    } else {
        theme::MUTED
    };
    let deny_color = if props.data.selected_allow {
        theme::MUTED
    } else {
        theme::ERROR
    };
    let allow_indicator = if props.data.selected_allow {
        "▸ "
    } else {
        "  "
    };
    let deny_indicator = if props.data.selected_allow {
        "  "
    } else {
        "▸ "
    };
    let args_display = props.data.arguments_preview(300, 12);

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 56,
                flex_direction: FlexDirection::Column,
                border_style: BorderStyle::Round,
                border_color: theme::WARN,
                background_color: theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
            ) {
                // Title
                Text(
                    content: "🔐 Tool Approval Required",
                    color: theme::WARN,
                    weight: Weight::Bold,
                )

                View(height: 1)

                // Tool name
                Text(
                    content: format!("Tool: {}", props.data.name),
                    color: theme::TEXT,
                    weight: Weight::Bold,
                )

                View(height: 1)

                // Arguments
                Text(
                    content: "Arguments:",
                    color: theme::MUTED,
                )
                Text(
                    content: args_display,
                    color: theme::TEXT,
                )

                View(height: 1)

                // Buttons
                View(
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::Center,
                    gap: 4,
                ) {
                    Text(
                        content: format!("{}Allow (y)", allow_indicator),
                        color: allow_color,
                        weight: Weight::Bold,
                    )
                    Text(
                        content: format!("{}Deny (n)", deny_indicator),
                        color: deny_color,
                        weight: Weight::Bold,
                    )
                }

                View(height: 1)

                // Hint
                Text(
                    content: "y allow · n/Esc deny · Tab toggle · Enter confirm",
                    color: theme::MUTED,
                )
            }
        }
    }
}
