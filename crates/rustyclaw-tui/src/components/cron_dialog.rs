// ── Cron dialog — scheduled jobs management overlay ─────────────────────────
//
// Opened with /cron; Esc closes. Shows all scheduled jobs with their
// expression, next run, last status, and actions.

use crate::theme;
use iocraft::prelude::*;

#[allow(dead_code)]
#[derive(Default, Props)]
pub struct CronDialogProps {
    pub data: Option<rustyclaw_view::CronPanelData>,
}

#[component]
pub fn CronDialog(props: &CronDialogProps) -> impl Into<AnyElement<'static>> {
    let mut rows: Vec<(String, String)> = Vec::new();

    if let Some(ref data) = props.data {
        if data.jobs.is_empty() {
            rows.push(("".into(), "(no scheduled jobs)".into()));
        } else {
            rows.push((
                "Summary".into(),
                format!(
                    "{} active / {} total",
                    data.active_count(),
                    data.total_count(),
                ),
            ));
            rows.push(("".into(), "".into()));

            for (i, job) in data.jobs.iter().enumerate() {
                let marker = if Some(i) == data.selected { ">" } else { " " };
                let status_symbol = if job.paused {
                    "[P]"
                } else {
                    match job.last_status.as_deref() {
                        Some("ok") | Some("success") => "[✓]",
                        Some("error") | Some("failed") => "[✕]",
                        Some("running") => "[~]",
                        _ => "[-]",
                    }
                };

                rows.push((
                    format!("{} {}", marker, job.name),
                    format!("{} {} (runs: {})", status_symbol, job.expr, job.run_count),
                ));

                let mut details = Vec::new();
                if let Some(ref next) = job.next_run {
                    details.push(format!("next: {}", next));
                }
                if let Some(ref last) = job.last_run {
                    details.push(format!("last: {}", last));
                }
                if !details.is_empty() {
                    rows.push(("".into(), format!("  {}", details.join(" | "))));
                }
            }
        }
    } else {
        rows.push(("".into(), "(cron system not initialised)".into()));
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
                    content: "Scheduled Jobs",
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
                    Text(content: "p ", color: theme::ACCENT_BRIGHT)
                    Text(content: "pause/resume  ", color: theme::MUTED)
                    Text(content: "r ", color: theme::ACCENT_BRIGHT)
                    Text(content: "run now  ", color: theme::MUTED)
                    Text(content: "d ", color: theme::ACCENT_BRIGHT)
                    Text(content: "delete", color: theme::MUTED)
                }
            }
        }
    }
}
