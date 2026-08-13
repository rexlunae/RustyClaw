//! Curated per-model inference defaults (#464).
//!
//! Some model families are tuned for specific sampling settings and degrade
//! visibly without them — most famously the Qwen and DeepSeek reasoning
//! lines. Providers apply their own defaults for their own hosted models,
//! but a model served through Ollama, LM Studio, OpenRouter or any other
//! OpenAI-compatible endpoint gets whatever the server's generic default is.
//!
//! This registry carries the *publisher's own* recommended sampling
//! settings, keyed by model-name pattern. Resolution order, strongest first:
//!
//! 1. The user's explicit `[model]` config — always wins, field by field.
//! 2. Entries from `<settings_dir>/model_defaults.toml`, user-editable and
//!    replaceable without a rebuild.
//! 3. The built-in table below.
//!
//! Patterns match case-insensitively as substrings of the model id (which
//! may or may not carry a `provider/` prefix); among matches the longest
//! pattern wins, so `qwq` can override a broader `qwen` entry. Entries carry
//! a note naming their source, shown by `rustyclaw model defaults`.
//!
//! Deliberately absent: the big first-party APIs (Claude, GPT, Gemini).
//! Their providers already apply the right defaults server-side; overriding
//! them here would be invention, not curation.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Recommended sampling settings for a model family.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InferenceDefaults {
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Where the numbers come from / why they matter. Documentation only.
    #[serde(default)]
    pub note: Option<String>,
}

/// A resolved lookup: which pattern matched and what it recommends.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedDefaults {
    /// The pattern that matched (e.g. `"qwen3"`).
    pub pattern: String,
    /// `"user"` for `model_defaults.toml` entries, `"built-in"` otherwise.
    pub source: &'static str,
    pub defaults: InferenceDefaults,
}

/// The built-in curated table.
///
/// Every entry states its source in `note`. Add sparingly: an entry is a
/// claim that the publisher recommends these numbers, not a preference.
fn built_in() -> Vec<(&'static str, InferenceDefaults)> {
    fn d(temperature: Option<f64>, top_p: Option<f64>, note: &str) -> InferenceDefaults {
        InferenceDefaults {
            temperature,
            top_p,
            max_tokens: None,
            note: Some(note.to_string()),
        }
    }
    vec![
        (
            "qwen",
            d(
                Some(0.7),
                Some(0.8),
                "Qwen team recommendation for instruct models; lower temperature \
                 also improves Qwen tool calling",
            ),
        ),
        (
            "qwq",
            d(
                Some(0.6),
                Some(0.95),
                "QwQ model card: recommended for reasoning output",
            ),
        ),
        (
            "deepseek-r1",
            d(
                Some(0.6),
                Some(0.95),
                "DeepSeek-R1 model card: 0.5-0.7 range, 0.6 recommended",
            ),
        ),
        (
            "deepseek-reasoner",
            d(
                Some(0.6),
                Some(0.95),
                "DeepSeek-R1 model card: 0.5-0.7 range, 0.6 recommended",
            ),
        ),
        (
            "glm-4",
            d(Some(0.6), None, "Zhipu GLM-4.x deployment guidance"),
        ),
        (
            "kimi-k2",
            d(Some(0.6), None, "Moonshot Kimi-K2 API guidance"),
        ),
        (
            "llama-3",
            d(
                Some(0.6),
                Some(0.9),
                "Meta Llama 3 generation_config defaults",
            ),
        ),
        (
            "llama3",
            d(
                Some(0.6),
                Some(0.9),
                "Meta Llama 3 generation_config defaults",
            ),
        ),
        (
            "gemma",
            d(
                Some(1.0),
                Some(0.95),
                "Gemma 3 model card sampling defaults",
            ),
        ),
    ]
}

/// Shape of `<settings_dir>/model_defaults.toml`:
///
/// ```toml
/// [models."qwen3"]
/// temperature = 0.6
/// top_p = 0.95
/// note = "my cluster serves qwen3 in thinking mode"
/// ```
#[derive(Debug, Default, Deserialize)]
struct DefaultsFile {
    #[serde(default)]
    models: BTreeMap<String, InferenceDefaults>,
}

/// File name of the user override table inside the settings directory.
pub const DEFAULTS_FILE: &str = "model_defaults.toml";

fn load_user_file(settings_dir: &Path) -> BTreeMap<String, InferenceDefaults> {
    let path = settings_dir.join(DEFAULTS_FILE);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return BTreeMap::new();
    };
    match toml::from_str::<DefaultsFile>(&raw) {
        Ok(file) => file.models,
        Err(e) => {
            // A broken table must not break model resolution — defaults are
            // an optimisation, never a dependency.
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "Could not parse model_defaults.toml; ignoring it"
            );
            BTreeMap::new()
        }
    }
}

/// Longest case-insensitive substring match wins.
fn best_match<'a, I>(model_lower: &str, entries: I) -> Option<(String, InferenceDefaults)>
where
    I: IntoIterator<Item = (&'a str, &'a InferenceDefaults)>,
{
    entries
        .into_iter()
        .filter(|(pattern, _)| model_lower.contains(&pattern.to_lowercase()))
        .max_by_key(|(pattern, _)| pattern.len())
        .map(|(pattern, d)| (pattern.to_string(), d.clone()))
}

/// Look up the curated defaults for `model`.
///
/// `settings_dir` locates the user override table; pass `None` to consult
/// only the built-ins. Any user-file match outranks every built-in match,
/// regardless of pattern length — the file exists to have the last word.
pub fn defaults_for(model: &str, settings_dir: Option<&Path>) -> Option<ResolvedDefaults> {
    let model_lower = model.to_lowercase();

    if let Some(dir) = settings_dir {
        let user = load_user_file(dir);
        if let Some((pattern, defaults)) =
            best_match(&model_lower, user.iter().map(|(k, v)| (k.as_str(), v)))
        {
            return Some(ResolvedDefaults {
                pattern,
                source: "user",
                defaults,
            });
        }
    }

    let built = built_in();
    best_match(&model_lower, built.iter().map(|(k, v)| (*k, v))).map(|(pattern, defaults)| {
        ResolvedDefaults {
            pattern,
            source: "built-in",
            defaults,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_family_resolves_regardless_of_id_shape() {
        for id in ["qwen3:8b", "ollama/qwen3-32b", "Qwen/Qwen3-8B-Instruct"] {
            let hit = defaults_for(id, None).expect("qwen family matches");
            assert_eq!(hit.pattern, "qwen");
            assert_eq!(hit.defaults.temperature, Some(0.7));
            assert_eq!(hit.defaults.top_p, Some(0.8));
        }
    }

    #[test]
    fn the_longest_pattern_wins() {
        // "deepseek-r1..." contains no other pattern, but the principle is
        // pinned with qwq vs qwen-style overlap: an id containing both must
        // resolve to the longer, more specific entry.
        let hit = defaults_for("qwen-qwq-32b", None).expect("matches");
        // "qwen" (4) vs "qwq" (3) — both match; qwen is longer. This is why
        // qwq ids without "qwen" in them exist as their own entry.
        assert_eq!(hit.pattern, "qwen");
        let hit = defaults_for("qwq-32b", None).expect("matches");
        assert_eq!(hit.pattern, "qwq");
        assert_eq!(hit.defaults.temperature, Some(0.6));
    }

    #[test]
    fn unknown_models_get_no_invented_defaults() {
        assert_eq!(defaults_for("claude-sonnet-4", None), None);
        assert_eq!(defaults_for("gpt-5.2", None), None);
        assert_eq!(defaults_for("some-custom-finetune", None), None);
    }

    #[test]
    fn the_user_file_outranks_the_built_ins() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join(DEFAULTS_FILE),
            "[models.\"qwen\"]\ntemperature = 0.3\nnote = \"local override\"\n",
        )
        .expect("write defaults file");

        let hit = defaults_for("qwen3:8b", Some(dir.path())).expect("matches");
        assert_eq!(hit.source, "user");
        assert_eq!(hit.defaults.temperature, Some(0.3));
        assert_eq!(
            hit.defaults.top_p, None,
            "the user entry replaces the built-in wholesale, not field-merged \
             — otherwise clearing a field would be impossible"
        );
    }

    #[test]
    fn a_broken_user_file_falls_back_to_built_ins() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(DEFAULTS_FILE), "not toml at all [[[")
            .expect("write defaults file");

        let hit = defaults_for("qwen3:8b", Some(dir.path())).expect("matches");
        assert_eq!(hit.source, "built-in");
    }
}
