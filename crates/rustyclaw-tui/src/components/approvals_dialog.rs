// ── Approvals dialog — pending approvals queue overlay ───────────────────────
//
// Opened with /approvals; Esc closes. Shows pending tool approval
// requests with batch allow/deny and "always allow" actions.

use crate::theme;
use iocraft::prelude::*;

#[derive(Default, Props)]
pub struct ApprovalsDialogProps {
    pub data: Option<rustyclaw_view::ApprovalsPanelData>,
}

#[component]
pub fn ApprovalsDialog(props: &ApprovalsDialogProps) -> impl Into<AnyElement<'static>> {
    let mut rows: Vec<(String, String)> = Vec::new();

    if let Some(ref data) = props.data {
        if data.approvals.is_empty() {
            rows.push(("".into(), "(no pending approvals)".into()));
        } else {
            rows.push((
                "Pending".into(),
                format!(
                    "{} total ({} selected)",
                    data.count(),
                    data.selected_count(),
                ),
            ));
            rows.push(("".into(), "".into()));

            for (i, approval) in data.approvals.iter().enumerate() {
                let marker = if Some(i) == data.cursor { ">" } else { " " };
                let check = if approval.selected { "[x]" } else { "[ ]" };

                rows.push((
                    format!("{} {} {}", marker, check, approval.tool_name),
                    approval.arguments_preview(50).to_string(),
                ));
                rows.push(("".into(), format!("  at {}", approval.requested_at)));
            }
        }
    } else {
        rows.push(("".into(), "(approvals system not available)".into()));
    }

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 70pct,
                max_height: 80pct,
                flex_direction: FlexDirection::Column,
                border_style: BorderStyle::Round,
                border_color: theme::INFO,
                background_color: theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
                overflow: Overflow::Hidden,
            ) {
                Text(
                    content: "Pending Approvals",
                    color: theme::INFO,
                    weight: Weight::Bold,
                )

                View(height: 1)

                #(rows.into_iter().enumerate().map(|(i, (label, value))| element! {
                    View(key: i as u32, flex_direction: FlexDirection::Row) {
                        Text(
                            content: if label.is_empty() { String::new() } else { format!("{:<20} ", label) },
                            color: theme::ACCENT_BRIGHT,
                        )
                        Text(content: value, color: theme::TEXT, wrap: TextWrap::Wrap)
                    }
                }))

                View(height: 1)

                View(flex_direction: FlexDirection::Row) {
                    Text(content: "Esc ", color: theme::ACCENT_BRIGHT)
                    Text(content: "close  ", color: theme::MUTED)
                    Text(content: "Space ", color: theme::ACCENT_BRIGHT)
                    Text(content: "select  ", color: theme::MUTED)
                    Text(content: "a ", color: theme::ACCENT_BRIGHT)
                    Text(content: "allow  ", color: theme::MUTED)
                    Text(content: "x ", color: theme::ACCENT_BRIGHT)
                    Text(content: "deny", color: theme::MUTED)
                }
            }
        }
    }
}
