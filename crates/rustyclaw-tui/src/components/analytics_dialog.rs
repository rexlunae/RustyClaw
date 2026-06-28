// ── Analytics dialog — token/usage dashboard overlay ─────────────────────────
//
// Opened with /analytics; Esc closes. Shows token usage totals and
// per-model/per-session breakdowns.

use crate::theme;
use iocraft::prelude::*;
use rustyclaw_view::analytics::UsageTotalsData;

#[derive(Default, Props)]
pub struct AnalyticsDialogProps {
    pub data: Option<rustyclaw_view::AnalyticsPanelData>,
}

#[component]
pub fn AnalyticsDialog(props: &AnalyticsDialogProps) -> impl Into<AnyElement<'static>> {
    let mut rows: Vec<(String, String)> = Vec::new();

    if let Some(ref data) = props.data {
        rows.push(("Period".into(), data.period.clone()));
        rows.push(("".into(), "".into()));

        // Totals section
        rows.push(("Requests".into(), data.totals.total_requests.to_string()));
        rows.push((
            "Input Tokens".into(),
            UsageTotalsData::tokens_display(data.totals.total_input_tokens),
        ));
        rows.push((
            "Output Tokens".into(),
            UsageTotalsData::tokens_display(data.totals.total_output_tokens),
        ));
        rows.push((
            "Avg Latency".into(),
            format!("{}ms", data.totals.avg_latency_ms()),
        ));
        rows.push(("".into(), "".into()));

        // Per-model breakdown
        if !data.per_model.is_empty() {
            rows.push(("── By Model ──".into(), "".into()));
            for m in &data.per_model {
                rows.push((
                    format!("{}/{}", m.provider, m.model),
                    format!(
                        "{} reqs | {} tokens | {}ms avg",
                        m.requests,
                        UsageTotalsData::tokens_display(m.total_tokens()),
                        m.avg_latency_ms,
                    ),
                ));
            }
            rows.push(("".into(), "".into()));
        }

        // Per-session breakdown
        if !data.per_session.is_empty() {
            rows.push(("── By Session ──".into(), "".into()));
            for s in &data.per_session {
                rows.push((
                    s.display_label().to_string(),
                    format!(
                        "{} reqs | {} tokens",
                        s.requests,
                        UsageTotalsData::tokens_display(s.input_tokens + s.output_tokens),
                    ),
                ));
            }
        }
    } else {
        rows.push(("".into(), "(analytics not available)".into()));
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
                max_height: 85pct,
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
                    content: "Usage Analytics",
                    color: theme::INFO,
                    weight: Weight::Bold,
                )

                View(height: 1)

                #(rows.into_iter().enumerate().map(|(i, (label, value))| element! {
                    View(key: i as u32, flex_direction: FlexDirection::Row) {
                        Text(
                            content: if label.is_empty() { String::new() } else { format!("{:<18} ", label) },
                            color: theme::ACCENT_BRIGHT,
                        )
                        Text(content: value, color: theme::TEXT, wrap: TextWrap::Wrap)
                    }
                }))

                View(height: 1)

                View(flex_direction: FlexDirection::Row) {
                    Text(content: "Esc ", color: theme::ACCENT_BRIGHT)
                    Text(content: "close", color: theme::MUTED)
                }
            }
        }
    }
}
