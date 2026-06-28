// ── Engines dialog — local engine and model management overlay ───────────────
//
// Opened with /engines; Esc closes. Shows engines with status badges,
// models per engine with load/size info, and pull progress.

use crate::theme;
use iocraft::prelude::*;

#[allow(dead_code)]
#[derive(Default, Props)]
pub struct EnginesDialogProps {
    pub data: Option<rustyclaw_view::EnginesPanelData>,
}

#[component]
pub fn EnginesDialog(props: &EnginesDialogProps) -> impl Into<AnyElement<'static>> {
    let mut rows: Vec<(String, String)> = Vec::new();

    if let Some(ref data) = props.data {
        if data.engines.is_empty() {
            rows.push(("".into(), "(no engines detected)".into()));
        } else {
            // Resource header
            if data.host_vram_bytes > 0 || data.host_ram_bytes > 0 {
                let ram = format_bytes(data.host_ram_bytes);
                let vram = format_bytes(data.host_vram_bytes);
                let gpu = data.host_gpu_name.as_deref().unwrap_or("unknown");
                rows.push((
                    "Host".into(),
                    format!("RAM: {} | VRAM: {} ({})", ram, vram, gpu),
                ));
                rows.push(("".into(), "".into()));
            }

            // Engine list
            for engine in &data.engines {
                let badge = engine.status_badge();
                let models_info = if engine.running {
                    format!(
                        "{} available, {} loaded",
                        engine.available_models, engine.loaded_models
                    )
                } else {
                    "\u{2014}".into()
                };
                rows.push((format!("{} [{}]", engine.display_name, badge), models_info));
                if let Some(ref ver) = engine.version {
                    rows.push(("  version".into(), ver.clone()));
                }
                if let Some(ref ep) = engine.endpoint {
                    rows.push(("  endpoint".into(), ep.clone()));
                }

                // Actions available
                let mut actions = Vec::new();
                if !engine.installed && engine.can("install") {
                    actions.push("install");
                }
                if engine.installed && !engine.running && engine.can("start") {
                    actions.push("start");
                }
                if engine.running && engine.can("stop") {
                    actions.push("stop");
                }
                if engine.running && engine.can("pull") {
                    actions.push("pull <model>");
                }
                if !actions.is_empty() {
                    rows.push(("  actions".into(), actions.join(" | ")));
                }
                rows.push(("".into(), "".into()));
            }

            // Selected engine's models
            if let Some(ref selected) = data.selected_engine {
                rows.push(("Models".into(), format!("({})", selected)));
                if data.models.is_empty() {
                    rows.push(("".into(), "(no models)".into()));
                } else {
                    for model in &data.models {
                        let loaded_mark = if model.loaded { "*" } else { " " };
                        let size = model.size_display();
                        let quant = model.quantization.as_deref().unwrap_or("");
                        rows.push((
                            format!(" {}{}", loaded_mark, model.name),
                            format!("{} {}", size, quant),
                        ));
                    }
                }
            }

            // Pull progress
            if let Some(ref progress) = data.pull_progress {
                rows.push(("".into(), "".into()));
                rows.push(("Pull".into(), progress.display()));
                let filled = (progress.pct() as usize) / 5;
                let empty = 20_usize.saturating_sub(filled);
                rows.push((
                    "".into(),
                    format!(
                        "[{}{}]",
                        "\u{2588}".repeat(filled),
                        "\u{2591}".repeat(empty)
                    ),
                ));
            }
        }
    } else {
        rows.push(("".into(), "(loading engine data\u{2026})".into()));
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
                    content: "Local Engines & Models",
                    color: theme::ACCENT,
                    weight: Weight::Bold,
                )

                View(height: 1)

                #(rows.into_iter().enumerate().map(|(i, (label, value))| element! {
                    View(key: i as u32, flex_direction: FlexDirection::Row) {
                        Text(
                            content: if label.is_empty() { String::new() } else { format!("{:<26} ", label) },
                            color: theme::ACCENT_BRIGHT,
                        )
                        Text(content: value, color: theme::TEXT, wrap: TextWrap::Wrap)
                    }
                }))

                View(height: 1)

                View(flex_direction: FlexDirection::Row) {
                    Text(content: "Esc ", color: theme::ACCENT_BRIGHT)
                    Text(content: "close  ", color: theme::MUTED)
                    Text(content: "Enter ", color: theme::ACCENT_BRIGHT)
                    Text(content: "select  ", color: theme::MUTED)
                    Text(content: "s ", color: theme::ACCENT_BRIGHT)
                    Text(content: "start/stop  ", color: theme::MUTED)
                    Text(content: "p ", color: theme::ACCENT_BRIGHT)
                    Text(content: "pull", color: theme::MUTED)
                }
            }
        }
    }
}

#[allow(dead_code)]
fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else if bytes >= 1_000_000 {
        format!("{:.0} MB", bytes as f64 / 1e6)
    } else {
        format!("{:.0} KB", bytes as f64 / 1e3)
    }
}
