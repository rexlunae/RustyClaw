// ── Engine parameter fields (TUI) ────────────────────────────────────────────
//
// The engines dialog edits the typed `EngineConfig` parameter fields
// (context window, device, huge pages, …) with +/- keys.  This module owns
// the field model: which fields each engine exposes, how a field's value
// renders, and how +/- (or x) adjusts it.  The desktop dialog mirrors these
// same fields as form inputs.

use rustyclaw_core::engines::EngineConfig;
use rustyclaw_view::LocalEngineData;

/// One editable parameter field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamField {
    /// Stable key: the `EngineConfig` field the parameter maps to.
    pub key: &'static str,
    /// Short label for the dialog row.
    pub label: &'static str,
    /// Increment for +/- on numeric fields.
    pub step: u32,
}

/// Context window in tokens (engine flag: `--n-ctx` / `--ctx-size` /
/// `--num-ctx`).  First +/- from unset lands on 4096; steps by 256.
pub const CONTEXT_LENGTH: ParamField = ParamField {
    key: "context_length",
    label: "Context window",
    step: 256,
};
/// Compute backend (`--device`): auto, cpu, metal, cuda.
pub const DEVICE: ParamField = ParamField {
    key: "device",
    label: "Device",
    step: 1,
};
/// Huge-page strategy (`--huge-pages`): off, transparent, 2mb, 1gb, huge.
pub const HUGE_PAGES: ParamField = ParamField {
    key: "huge_pages",
    label: "Huge pages",
    step: 1,
};
/// Require the model file to be memory-mappable (`--mmap`).
pub const MMAP: ParamField = ParamField {
    key: "mmap",
    label: "Mmap",
    step: 1,
};
/// Optimise mapping for a model far larger than RAM (`--lazy-weights`).
pub const LAZY_WEIGHTS: ParamField = ParamField {
    key: "lazy_weights",
    label: "Lazy weights",
    step: 1,
};
/// Cap on generated tokens per request (`--max-output-tokens`).  First
/// +/- from unset lands on 4096; steps by 128.
pub const MAX_OUTPUT_TOKENS: ParamField = ParamField {
    key: "max_output_tokens",
    label: "Max output tokens",
    step: 128,
};
/// Max concurrent generations (`--max-concurrency`).  Steps by 1.
pub const MAX_CONCURRENCY: ParamField = ParamField {
    key: "max_concurrency",
    label: "Max concurrency",
    step: 1,
};
/// The model served at startup; cycles through the local model list.
pub const DEFAULT_MODEL: ParamField = ParamField {
    key: "default_model",
    label: "Default model",
    step: 1,
};
/// Start the engine with the gateway.
pub const AUTO_START: ParamField = ParamField {
    key: "auto_start",
    label: "Auto-start",
    step: 1,
};

/// The fields the engines dialog exposes for `engine`, in edit order.
pub fn fields_for(engine: &LocalEngineData) -> Vec<ParamField> {
    let mut fields = Vec::new();
    if engine.supports_context_length() {
        fields.push(CONTEXT_LENGTH);
    }
    if engine.supports_joshua_parameters() {
        fields.push(DEVICE);
        fields.push(HUGE_PAGES);
        fields.push(MMAP);
        fields.push(LAZY_WEIGHTS);
        fields.push(MAX_OUTPUT_TOKENS);
        fields.push(MAX_CONCURRENCY);
    }
    if engine.supports_default_model() {
        fields.push(DEFAULT_MODEL);
    }
    if engine.can("start") {
        fields.push(AUTO_START);
    }
    fields
}

/// Render a field's current value for the dialog row.
pub fn field_value(field: &ParamField, cfg: &EngineConfig, models: &[String]) -> String {
    let value = match field.key {
        "context_length" => cfg
            .context_length
            .map(|v| v.to_string())
            .unwrap_or_else(|| "default".into()),
        "device" => cfg
            .device
            .clone()
            .unwrap_or_else(|| "default (auto)".into()),
        "huge_pages" => cfg
            .huge_pages
            .clone()
            .unwrap_or_else(|| "default (off)".into()),
        "mmap" => on_off(cfg.mmap),
        "lazy_weights" => on_off(cfg.lazy_weights),
        "max_output_tokens" => cfg
            .max_output_tokens
            .map(|v| v.to_string())
            .unwrap_or_else(|| "default (4096)".into()),
        "max_concurrency" => cfg
            .max_concurrency
            .map(|v| v.to_string())
            .unwrap_or_else(|| "default (CPU count)".into()),
        "default_model" => cfg.default_model.clone().unwrap_or_else(|| "none".into()),
        "auto_start" => on_off(cfg.auto_start),
        _ => String::new(),
    };
    // `default_model` cycles through the loaded model names; point at the
    // + key when it is unset and models exist.
    if field.key == "default_model" && cfg.default_model.is_none() && !models.is_empty() {
        format!("{value} (+ to pick)")
    } else {
        value
    }
}

fn on_off(v: bool) -> String {
    if v { "on".into() } else { "off".into() }
}

/// Adjust a field by `delta` (+1 / -1 keypresses).  Numeric fields step by
/// `field.step`; selects cycle through their options (None included); toggles
/// set on (+) or off (-); `default_model` cycles through `models` (None
/// included, and a no-op while the model list is empty so the saved choice
/// is never erased by a pick with nothing to pick).
pub fn adjust(field: &ParamField, cfg: &mut EngineConfig, delta: i32, models: &[String]) {
    match field.key {
        "context_length" => adjust_num(&mut cfg.context_length, delta, field.step),
        "device" => cfg.device = cycle(&cfg.device, &["auto", "cpu", "metal", "cuda"], delta),
        "huge_pages" => {
            cfg.huge_pages = cycle(
                &cfg.huge_pages,
                &["off", "transparent", "2mb", "1gb", "huge"],
                delta,
            );
        }
        "mmap" => bool_adjust(&mut cfg.mmap, delta),
        "lazy_weights" => bool_adjust(&mut cfg.lazy_weights, delta),
        "max_output_tokens" => adjust_num(&mut cfg.max_output_tokens, delta, field.step),
        "max_concurrency" => adjust_num(&mut cfg.max_concurrency, delta, field.step),
        "default_model" => {
            // With no model list there is nothing to pick from: cycling
            // would land on the None-only option and silently erase the
            // saved choice, so changing the key is a no-op instead.
            if models.is_empty() {
                return;
            }
            let mut options: Vec<Option<String>> = vec![None];
            options.extend(models.iter().cloned().map(Some));
            let idx = options
                .iter()
                .position(|o| *o == cfg.default_model)
                .unwrap_or(0);
            let next = (idx as i32 + delta).rem_euclid(options.len() as i32) as usize;
            cfg.default_model = options[next].clone();
        }
        "auto_start" => bool_adjust(&mut cfg.auto_start, delta),
        _ => {}
    }
}

/// Clear a field back to its default (None / off).
pub fn clear(field: &ParamField, cfg: &mut EngineConfig) {
    match field.key {
        "context_length" => cfg.context_length = None,
        "device" => cfg.device = None,
        "huge_pages" => cfg.huge_pages = None,
        "mmap" => cfg.mmap = false,
        "lazy_weights" => cfg.lazy_weights = false,
        "max_output_tokens" => cfg.max_output_tokens = None,
        "max_concurrency" => cfg.max_concurrency = None,
        "default_model" => cfg.default_model = None,
        "auto_start" => cfg.auto_start = false,
        _ => {}
    }
}

fn adjust_num(slot: &mut Option<u32>, delta: i32, step: u32) {
    // First +/- from "default" lands on a sensible base.
    let base = match step {
        256 => 4096,
        128 => 4096,
        _ => 1,
    };
    match slot {
        Some(current) => {
            let next = if delta > 0 {
                current.saturating_add(step)
            } else {
                current.saturating_sub(step).max(step.min(*current))
            };
            *slot = Some(next);
        }
        None => *slot = Some(base),
    }
}

fn bool_adjust(slot: &mut bool, delta: i32) {
    *slot = delta > 0;
}

fn cycle(current: &Option<String>, options: &[&str], delta: i32) -> Option<String> {
    // Index 0 is "unset"; the remaining slots are the options in order.
    let idx = match current {
        Some(v) => options
            .iter()
            .position(|o| o == v)
            .map(|i| i + 1)
            .unwrap_or(0),
        None => 0,
    };
    let total = options.len() + 1;
    let next = (idx as i32 + delta).rem_euclid(total as i32) as usize;
    if next == 0 {
        None
    } else {
        Some(options[next - 1].to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(id: &str, can_start: bool) -> LocalEngineData {
        LocalEngineData {
            id: id.into(),
            display_name: id.into(),
            installed: false,
            running: false,
            version: None,
            endpoint: None,
            available_models: 0,
            loaded_models: 0,
            caps: rustyclaw_view::EngineCapsData {
                can_start,
                ..Default::default()
            },
            config: EngineConfig::default(),
        }
    }

    #[test]
    fn fields_are_engine_specific() {
        let joshua = engine("joshua", true);
        let keys: Vec<&str> = fields_for(&joshua).iter().map(|f| f.key).collect();
        assert_eq!(
            keys,
            vec![
                "context_length",
                "device",
                "huge_pages",
                "mmap",
                "lazy_weights",
                "max_output_tokens",
                "max_concurrency",
                "default_model",
                "auto_start",
            ]
        );

        let ollama = engine("ollama", true);
        let keys: Vec<&str> = fields_for(&ollama).iter().map(|f| f.key).collect();
        assert_eq!(keys, vec!["context_length", "auto_start"]);

        let lmstudio = engine("lmstudio", false);
        assert!(fields_for(&lmstudio).is_empty());
    }

    #[test]
    fn numeric_adjust_steps_from_default() {
        let mut cfg = EngineConfig::default();
        adjust(&CONTEXT_LENGTH, &mut cfg, 1, &[]);
        assert_eq!(cfg.context_length, Some(4096));
        adjust(&CONTEXT_LENGTH, &mut cfg, 1, &[]);
        assert_eq!(cfg.context_length, Some(4352));
        adjust(&CONTEXT_LENGTH, &mut cfg, -1, &[]);
        assert_eq!(cfg.context_length, Some(4096));
    }

    #[test]
    fn selects_cycle_including_unset() {
        let mut cfg = EngineConfig::default();
        adjust(&DEVICE, &mut cfg, 1, &[]);
        assert_eq!(cfg.device.as_deref(), Some("auto"));
        adjust(&DEVICE, &mut cfg, 1, &[]);
        assert_eq!(cfg.device.as_deref(), Some("cpu"));
        // Back past the start wraps to the last option.
        adjust(&DEVICE, &mut cfg, -4, &[]);
        assert_eq!(cfg.device.as_deref(), Some("metal"));
        adjust(&DEVICE, &mut cfg, 1, &[]);
        assert_eq!(cfg.device.as_deref(), Some("cuda"));
        adjust(&DEVICE, &mut cfg, 1, &[]);
        assert_eq!(cfg.device, None);
    }

    #[test]
    fn default_model_cycles_through_local_models() {
        let models = vec!["a.gguf".to_string(), "b.gguf".to_string()];
        let mut cfg = EngineConfig::default();
        adjust(&DEFAULT_MODEL, &mut cfg, 1, &models);
        assert_eq!(cfg.default_model.as_deref(), Some("a.gguf"));
        adjust(&DEFAULT_MODEL, &mut cfg, 1, &models);
        assert_eq!(cfg.default_model.as_deref(), Some("b.gguf"));
        adjust(&DEFAULT_MODEL, &mut cfg, 1, &models);
        assert_eq!(cfg.default_model, None);
        // -1 from unset wraps to the last model.
        adjust(&DEFAULT_MODEL, &mut cfg, -1, &models);
        assert_eq!(cfg.default_model.as_deref(), Some("b.gguf"));
    }

    #[test]
    fn default_model_is_a_noop_with_an_empty_model_list() {
        // With nothing to pick, +/- must not erase the saved choice (the
        // cycle would otherwise land on the None-only option).
        let mut cfg = EngineConfig {
            default_model: Some("a.gguf".into()),
            ..Default::default()
        };
        adjust(&DEFAULT_MODEL, &mut cfg, 1, &[]);
        assert_eq!(cfg.default_model.as_deref(), Some("a.gguf"));
        adjust(&DEFAULT_MODEL, &mut cfg, -1, &[]);
        assert_eq!(cfg.default_model.as_deref(), Some("a.gguf"));
    }

    #[test]
    fn clear_resets_to_defaults() {
        let mut cfg = EngineConfig {
            context_length: Some(8192),
            mmap: true,
            device: Some("cuda".into()),
            ..Default::default()
        };
        clear(&CONTEXT_LENGTH, &mut cfg);
        clear(&MMAP, &mut cfg);
        clear(&DEVICE, &mut cfg);
        assert_eq!(cfg.context_length, None);
        assert!(!cfg.mmap);
        assert_eq!(cfg.device, None);
    }
}
