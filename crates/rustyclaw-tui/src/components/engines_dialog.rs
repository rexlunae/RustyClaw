// ── Engines dialog — local engine and model management overlay ───────────────
//
// Opened with /engines; Esc closes. One tab per engine: the tab strip lists
// every detected engine (active one highlighted), and the body shows the
// active engine's status, models, live install output, and pull progress.
// ←/→ (or Tab) switches engines.
//
// The body also renders the engine's parameter fields (context window,
// device, huge pages, …).  Pressing p enters parameter-edit mode: ←/→ cycles
// the focused field, +/- adjusts it (x clears it back to default), Enter
// saves the draft via EngineConfigSet, Esc discards it.

use std::collections::HashMap;

use crate::components::engines_params;
use crate::theme;
use iocraft::prelude::*;

#[derive(Default, Props)]
pub struct EnginesDialogProps {
    pub data: Option<rustyclaw_view::EnginesPanelData>,
    pub params_edit: bool,
    pub params_cursor: usize,
    pub params_drafts: HashMap<String, rustyclaw_core::engines::EngineConfig>,
    /// Outcome of the last engine model action (engine, ok, message).
    pub action_result: Option<(String, bool, String)>,
}

/// One label/value detail row in the active engine's body.
struct Row {
    label: String,
    value: String,
    /// Highlight the row (the focused parameter field in edit mode).
    highlighted: bool,
}

impl Row {
    fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            highlighted: false,
        }
    }

    fn highlight(mut self) -> Self {
        self.highlighted = true;
        self
    }
}

#[component]
pub fn EnginesDialog(props: &EnginesDialogProps) -> impl Into<AnyElement<'static>> {
    // Tab strip: (display_name, status_badge, is_active) per engine.
    let mut tabs: Vec<(String, &'static str, bool)> = Vec::new();
    // Detail rows for the active engine.
    let mut rows: Vec<Row> = Vec::new();
    // Live install output lines for the active engine (already bounded).
    let mut install_status: Option<&'static str> = None;
    let mut install_lines: Vec<String> = Vec::new();
    let mut empty_note: Option<String> = None;

    match &props.data {
        None => empty_note = Some("(loading engine data\u{2026})".into()),
        Some(data) if data.engines.is_empty() => empty_note = Some("(no engines detected)".into()),
        Some(data) => {
            let active_idx = data.active_index();
            for (i, engine) in data.engines.iter().enumerate() {
                tabs.push((
                    engine.display_name.clone(),
                    engine.status_badge(),
                    i == active_idx,
                ));
            }

            // Host resource header (shared context above the tab body).
            if data.host_vram_bytes > 0 || data.host_ram_bytes > 0 {
                let ram = format_bytes(data.host_ram_bytes);
                let vram = format_bytes(data.host_vram_bytes);
                let gpu = data.host_gpu_name.as_deref().unwrap_or("unknown");
                rows.push(Row::new(
                    "Host",
                    format!("RAM: {} | VRAM: {} ({})", ram, vram, gpu),
                ));
                rows.push(Row::new("", ""));
            }

            if let Some(engine) = data.active_engine() {
                rows.push(Row::new("Status", engine.status_badge()));
                if let Some(ref ver) = engine.version {
                    rows.push(Row::new("Version", ver.clone()));
                }
                if let Some(ref ep) = engine.endpoint {
                    rows.push(Row::new("Endpoint", ep.clone()));
                }
                if engine.running {
                    rows.push(Row::new(
                        "Models",
                        format!(
                            "{} available, {} loaded",
                            engine.available_models, engine.loaded_models
                        ),
                    ));
                }

                // Actions available for this engine.
                let mut actions = Vec::new();
                if !engine.installed && engine.can("install") {
                    actions.push("i install");
                }
                if engine.installed && !engine.running && engine.can("start") {
                    actions.push("s start");
                }
                if engine.running && engine.can("stop") {
                    actions.push("s stop");
                }
                if engine.running && engine.can("pull") {
                    actions.push("pull <model>");
                }
                if !actions.is_empty() {
                    rows.push(Row::new("Actions", actions.join(" | ")));
                }

                // ── Parameters ─────────────────────────────────────────
                let fields = engines_params::fields_for(engine);
                if !fields.is_empty() {
                    rows.push(Row::new("", ""));
                    let draft = props
                        .params_drafts
                        .get(&engine.id)
                        .unwrap_or(&engine.config);
                    let models: Vec<String> = data.models.iter().map(|m| m.name.clone()).collect();
                    rows.push(Row::new(
                        "Parameters",
                        if props.params_edit {
                            "(edit mode \u{2014} see footer)"
                        } else {
                            "p to edit"
                        }
                        .to_string(),
                    ));
                    for (i, field) in fields.iter().enumerate() {
                        let focused = props.params_edit && i == props.params_cursor;
                        let marker = if focused { "\u{2192}" } else { " " };
                        let mut row = Row::new(
                            format!(" {marker} {}", field.label),
                            engines_params::field_value(field, draft, &models),
                        );
                        if focused {
                            row = row.highlight();
                        }
                        rows.push(row);
                    }
                }

                // Models for the active (selected) engine.
                if data.selected_engine.as_deref() == Some(engine.id.as_str()) {
                    rows.push(Row::new("", ""));
                    if data.models.is_empty() {
                        rows.push(Row::new("Models", "(none — press Enter to load)"));
                    } else {
                        for model in &data.models {
                            let loaded_mark = if model.loaded { "*" } else { " " };
                            let size = model.size_display();
                            let quant = model.quantization.as_deref().unwrap_or("");
                            let fit = if !model.fits_host { " \u{26a0}" } else { "" };
                            rows.push(Row::new(
                                format!(" {}{}{}", loaded_mark, model.name, fit),
                                format!("{} {}", size, quant),
                            ));
                            if let Some(warning) = model.fit_warning() {
                                rows.push(Row::new("   \u{26a0}", warning.to_string()));
                            }
                        }
                    }
                }

                // Outcome of the last Load/Unload on this engine, so the
                // action always answers visibly.
                if let Some((result_engine, ok, message)) = &props.action_result {
                    if result_engine == &engine.id {
                        rows.push(Row::new("", ""));
                        rows.push(Row::new(
                            if *ok { "Result" } else { "Error" },
                            message.clone(),
                        ));
                    }
                }

                // Live install output for this engine's tab.
                if let Some(output) = data.active_install_output() {
                    install_status = Some(output.status_line());
                    install_lines = output.tail(8).to_vec();
                }

                // Pull progress (panel-wide, but shown on its engine's tab).
                if let Some(ref progress) = data.pull_progress {
                    if progress.engine == engine.id {
                        rows.push(Row::new("", ""));
                        rows.push(Row::new("Pull", progress.display()));
                        let filled = (progress.pct() as usize) / 5;
                        let empty = 20_usize.saturating_sub(filled);
                        rows.push(Row::new(
                            "",
                            format!(
                                "[{}{}]",
                                "\u{2588}".repeat(filled),
                                "\u{2591}".repeat(empty)
                            ),
                        ));
                    }
                }
            }
        }
    }

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 72pct,
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

                // Tab strip.
                #(if tabs.is_empty() {
                    element! { View() }.into_any()
                } else {
                    element! {
                        View(flex_direction: FlexDirection::Row, flex_wrap: FlexWrap::Wrap) {
                            #(tabs.into_iter().enumerate().map(|(i, (name, badge, active))| {
                                let label = format!(" {} ({}) ", name, badge);
                                element! {
                                    View(
                                        key: i as u32,
                                        margin_right: 1,
                                        background_color: if active { theme::ACCENT } else { theme::BG_CODE },
                                    ) {
                                        Text(
                                            content: label,
                                            color: if active { theme::BG_MAIN } else { theme::TEXT_DIM },
                                            weight: if active { Weight::Bold } else { Weight::Normal },
                                        )
                                    }
                                }
                            }))
                        }
                    }.into_any()
                })

                View(height: 1)

                // Body: either the empty note or the active engine's detail rows.
                #(if let Some(note) = empty_note {
                    element! {
                        Text(content: note, color: theme::MUTED)
                    }.into_any()
                } else {
                    element! {
                        View(flex_direction: FlexDirection::Column) {
                            #(rows.into_iter().enumerate().map(|(i, row)| element! {
                                View(key: i as u32, flex_direction: FlexDirection::Row) {
                                    Text(
                                        content: if row.label.is_empty() { String::new() } else { format!("{:<16} ", row.label) },
                                        color: theme::ACCENT_BRIGHT,
                                        weight: if row.highlighted { Weight::Bold } else { Weight::Normal },
                                    )
                                    Text(
                                        content: row.value,
                                        color: if row.highlighted { theme::ACCENT_BRIGHT } else { theme::TEXT },
                                        wrap: TextWrap::Wrap,
                                        weight: if row.highlighted { Weight::Bold } else { Weight::Normal },
                                    )
                                }
                            }))
                        }
                    }.into_any()
                })

                // Live install output panel.
                #(if let Some(status) = install_status {
                    element! {
                        View(flex_direction: FlexDirection::Column, margin_top: 1) {
                            Text(content: format!("Install \u{2014} {status}"), color: theme::WARN, weight: Weight::Bold)
                            #(install_lines.into_iter().enumerate().map(|(i, line)| element! {
                                Text(key: i as u32, content: line, color: theme::TEXT_DIM, wrap: TextWrap::Wrap)
                            }))
                        }
                    }.into_any()
                } else {
                    element! { View() }.into_any()
                })

                View(height: 1)

                View(flex_direction: FlexDirection::Row, flex_wrap: FlexWrap::Wrap) {
                    Text(content: "Esc ", color: theme::ACCENT_BRIGHT)
                    Text(content: if props.params_edit { "cancel  " } else { "close  " }, color: theme::MUTED)
                    Text(content: "\u{2190}/\u{2192} ", color: theme::ACCENT_BRIGHT)
                    Text(content: if props.params_edit { "field  " } else { "switch engine  " }, color: theme::MUTED)
                    #(if props.params_edit {
                        element! {
                            View(flex_direction: FlexDirection::Row) {
                                Text(content: "Enter ", color: theme::ACCENT_BRIGHT)
                                Text(content: "save  ", color: theme::MUTED)
                                Text(content: "+/- ", color: theme::ACCENT_BRIGHT)
                                Text(content: "adjust  ", color: theme::MUTED)
                                Text(content: "x ", color: theme::ACCENT_BRIGHT)
                                Text(content: "clear  ", color: theme::MUTED)
                            }
                        }.into_any()
                    } else {
                        element! {
                            View(flex_direction: FlexDirection::Row) {
                                Text(content: "Enter ", color: theme::ACCENT_BRIGHT)
                                Text(content: "models  ", color: theme::MUTED)
                                Text(content: "s ", color: theme::ACCENT_BRIGHT)
                                Text(content: "start/stop  ", color: theme::MUTED)
                                Text(content: "p ", color: theme::ACCENT_BRIGHT)
                                Text(content: "params  ", color: theme::MUTED)
                                Text(content: "i ", color: theme::ACCENT_BRIGHT)
                                Text(content: "install  ", color: theme::MUTED)
                                Text(content: "r ", color: theme::ACCENT_BRIGHT)
                                Text(content: "refresh", color: theme::MUTED)
                            }
                        }.into_any()
                    })
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
