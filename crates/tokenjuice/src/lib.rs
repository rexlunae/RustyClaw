//! TokenJuice — rule-driven tool-output compression.
//!
//! Sits between a tool result and the LLM context window. Each rule in the
//! three-layer overlay (builtin / user / project) declares a match (by tool
//! name, argv, or command substring) and a reduction strategy (line filters,
//! transforms, head/tail summarize, ANSI strip). The highest-priority matching
//! rule wins; ties resolve to deeper overlay layers.
//!
//! # Quick start
//!
//! ```
//! use tokenjuice::{TokenJuice, ToolInput};
//!
//! let tj = TokenJuice::builtin();
//! let input = ToolInput {
//!     tool_name: "execute_command",
//!     command: Some("git status"),
//!     argv: &["git".into(), "status".into()],
//!     output: "modified: foo.rs\nOn branch main\n",
//! };
//! let compact = tj.compact(&input);
//! assert!(compact.text.contains("modified: foo.rs"));
//! ```
//!
//! # Adding rules
//!
//! Drop a JSON file in `~/.config/tokenjuice/rules/` (user layer) or
//! `<project>/.tokenjuice/rules/` (project layer). Both layers override the
//! built-in rule with the same `id`.

mod builtin;
mod classify;
mod compile;
mod overlay;
mod reduce;
mod rule;

pub use classify::Classification;
pub use compile::{CompileError, CompiledCounter, CompiledRule};
pub use overlay::LoadError;
pub use reduce::NamedCount;
pub use rule::{
    JsonRule, LayeredRule, RuleCounter, RuleFilters, RuleMatch, RuleOrigin, RuleSummarize,
    RuleTransforms,
};

use std::path::Path;

/// Input to the classification + reduction pipeline.
///
/// Construct from whatever your tool layer knows. `tool_name` is always
/// available; the rest can be `None`/empty if the tool isn't a shell command.
#[derive(Debug)]
pub struct ToolInput<'a> {
    /// Agent-facing tool name, e.g. `execute_command`, `web_fetch`.
    pub tool_name: &'a str,
    /// Raw command line, when available (used by `commandIncludes` matchers).
    pub command: Option<&'a str>,
    /// Tokenized argv, when available (used by `argv0`, `argvIncludes`,
    /// `gitSubcommands` matchers).
    pub argv: &'a [String],
    /// The raw tool output to compress.
    pub output: &'a str,
}

/// Result of a `compact` call.
#[derive(Debug)]
pub struct CompactResult {
    /// The reduced output. Always safe to feed to the LLM in place of the raw.
    pub text: String,
    /// Length of the original output in bytes.
    pub raw_chars: usize,
    /// Length of the reduced output in bytes.
    pub reduced_chars: usize,
    /// Which rule family matched, if any. `None` means no rule matched and
    /// the original text was returned unchanged.
    pub family: Option<String>,
    /// Rule id of the rule that fired (if any).
    pub rule_id: Option<String>,
    /// Named counters the rule emitted (e.g. `{warnings: 12, errors: 1}`).
    pub counters: Vec<NamedCount>,
}

impl CompactResult {
    /// Ratio of `reduced_chars / raw_chars`, clamped to `[0.0, 1.0]`. Returns
    /// `1.0` for empty input.
    pub fn ratio(&self) -> f64 {
        if self.raw_chars == 0 {
            return 1.0;
        }
        (self.reduced_chars as f64 / self.raw_chars as f64).clamp(0.0, 1.0)
    }
}

/// The compiled rule overlay. Cheap to clone? No — keep one and share by `&`.
#[derive(Debug)]
pub struct TokenJuice {
    rules: Vec<CompiledRule>,
}

impl TokenJuice {
    /// Construct with only the built-in rules.
    pub fn builtin() -> Self {
        let layered = builtin::builtin_rules()
            .into_iter()
            .map(|r| r.into_layered(RuleOrigin::Builtin, None))
            .collect::<Vec<_>>();
        Self::from_layered_unchecked(layered)
    }

    /// Construct with the three-layer overlay applied. Missing directories
    /// are silently ignored (so callers can pass paths that may or may not
    /// exist yet).
    pub fn with_layers(
        user_dir: Option<&Path>,
        project_dir: Option<&Path>,
    ) -> Result<Self, TokenJuiceError> {
        let layered = overlay::load_layered(user_dir, project_dir)?;
        Self::from_layered(layered)
    }

    /// Build from an explicit list of layered rules. Useful for testing.
    pub fn from_layered(layered: Vec<LayeredRule>) -> Result<Self, TokenJuiceError> {
        let mut compiled = Vec::with_capacity(layered.len());
        for l in layered {
            compiled.push(CompiledRule::compile(l)?);
        }
        Ok(Self { rules: compiled })
    }

    fn from_layered_unchecked(layered: Vec<LayeredRule>) -> Self {
        // Built-in rules are author-controlled; compile failures are bugs.
        let mut compiled = Vec::with_capacity(layered.len());
        for l in layered {
            compiled.push(
                CompiledRule::compile(l).expect("builtin tokenjuice rule failed to compile"),
            );
        }
        Self { rules: compiled }
    }

    /// Number of compiled rules. Useful for sanity-checking overlay loading.
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Run the pipeline. If no rule matches, the original text passes through
    /// unchanged.
    pub fn compact(&self, input: &ToolInput<'_>) -> CompactResult {
        let raw_chars = input.output.len();
        let Some(c) = classify::classify(&self.rules, input) else {
            return CompactResult {
                text: input.output.to_string(),
                raw_chars,
                reduced_chars: raw_chars,
                family: None,
                rule_id: None,
                counters: vec![],
            };
        };
        let out = reduce::reduce(c.rule, input.output);
        let reduced_chars = out.text.len();
        CompactResult {
            text: out.text,
            raw_chars,
            reduced_chars,
            family: Some(c.family),
            rule_id: Some(c.rule.layered.rule.id.clone()),
            counters: out.counters,
        }
    }
}

impl Default for TokenJuice {
    fn default() -> Self {
        Self::builtin()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TokenJuiceError {
    #[error(transparent)]
    Load(#[from] LoadError),
    #[error(transparent)]
    Compile(#[from] CompileError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn builtin_loads_and_compiles() {
        let tj = TokenJuice::builtin();
        assert!(tj.rule_count() >= 5);
    }

    #[test]
    fn git_status_drops_chrome() {
        let tj = TokenJuice::builtin();
        let raw = "On branch main\nYour branch is up to date with 'origin/main'.\n\nChanges not staged for commit:\n  (use \"git add <file>...\" to update what will be committed)\n  (use \"git restore <file>...\" to discard changes in working directory)\n\tmodified:   src/lib.rs\n\tmodified:   src/main.rs\n\nno changes added to commit (use \"git add\" and/or \"git commit -a\")\n";
        let av = argv(&["git", "status"]);
        let res = tj.compact(&ToolInput {
            tool_name: "execute_command",
            command: Some("git status"),
            argv: &av,
            output: raw,
        });
        assert_eq!(res.family.as_deref(), Some("git"));
        assert!(res.text.contains("modified:   src/lib.rs"));
        assert!(!res.text.contains("On branch"));
        assert!(!res.text.contains("to discard"));
        assert!(res.reduced_chars < res.raw_chars);
    }

    #[test]
    fn cargo_build_drops_compiling_lines() {
        let tj = TokenJuice::builtin();
        let raw = "   Compiling foo v0.1.0\n   Compiling bar v0.2.0\nwarning: unused variable: `x`\n  --> src/lib.rs:5:9\nerror[E0001]: kaboom\n   Compiling baz v0.3.0\n    Finished `dev` profile in 12.3s\n";
        let av = argv(&["cargo", "build"]);
        let res = tj.compact(&ToolInput {
            tool_name: "execute_command",
            command: Some("cargo build"),
            argv: &av,
            output: raw,
        });
        assert_eq!(res.family.as_deref(), Some("cargo"));
        assert!(!res.text.contains("Compiling foo"));
        assert!(res.text.contains("warning: unused variable"));
        assert!(res.text.contains("error[E0001]"));
        assert!(res.text.contains("Finished"));

        let warnings = res.counters.iter().find(|c| c.name == "warnings").unwrap();
        assert_eq!(warnings.count, 1);
        let errors = res.counters.iter().find(|c| c.name == "errors").unwrap();
        assert_eq!(errors.count, 1);
    }

    #[test]
    fn unmatched_tool_passes_through_via_generic_rule() {
        let tj = TokenJuice::builtin();
        let av = Vec::<String>::new();
        let raw = "hello world\n";
        let res = tj.compact(&ToolInput {
            tool_name: "some_unknown_tool",
            command: None,
            argv: &av,
            output: raw,
        });
        // Generic rule still applies (low priority), but with a short input it
        // just trims edges so the text equals the original.
        assert!(res.text.contains("hello world"));
    }

    #[test]
    fn user_layer_overrides_builtin() {
        let user_rule = JsonRule {
            id: "tokenjuice.builtin.git.status".into(),
            family: "git".into(),
            description: None,
            priority: 100,
            r#match: RuleMatch {
                git_subcommands: vec!["status".into()],
                ..Default::default()
            },
            filters: RuleFilters {
                skip_patterns: vec!["modified".into()],
                keep_patterns: vec![],
            },
            transforms: RuleTransforms::default(),
            summarize: None,
            counters: vec![],
        };
        let layered = vec![
            // Builtin rule with same id.
            builtin::builtin_rules()
                .into_iter()
                .find(|r| r.id == "tokenjuice.builtin.git.status")
                .unwrap()
                .into_layered(RuleOrigin::Builtin, None),
            user_rule.into_layered(RuleOrigin::User, Some("user.json".into())),
        ];
        let tj = TokenJuice::from_layered(layered).unwrap();
        let av = argv(&["git", "status"]);
        let raw = "modified:   foo\nuntracked:  bar\n";
        let res = tj.compact(&ToolInput {
            tool_name: "execute_command",
            command: Some("git status"),
            argv: &av,
            output: raw,
        });
        // The user rule drops "modified" lines (built-in keeps them).
        assert!(!res.text.contains("modified:   foo"));
        assert!(res.text.contains("untracked:  bar"));
    }

    #[test]
    fn ratio_clamped() {
        let res = CompactResult {
            text: "x".to_string(),
            raw_chars: 0,
            reduced_chars: 0,
            family: None,
            rule_id: None,
            counters: vec![],
        };
        assert_eq!(res.ratio(), 1.0);
    }
}
