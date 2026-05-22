//! Rule schema. Mirrors the JSON shape used by vincentkoc/tokenjuice so
//! existing rule files port over with minimal edits.

use serde::{Deserialize, Serialize};

/// Where a rule was loaded from. Later origins win in the overlay merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleOrigin {
    Builtin,
    User,
    Project,
}

impl RuleOrigin {
    pub(crate) fn precedence(self) -> u8 {
        match self {
            Self::Builtin => 0,
            Self::User => 1,
            Self::Project => 2,
        }
    }
}

/// Match conditions. A rule matches when *every* specified field matches.
/// Within a field, `Any` variants are ORed and `Includes` (without `Any`)
/// are ANDed across the inner array.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleMatch {
    /// Match against the agent-side tool name (e.g. `execute_command`, `web_fetch`).
    #[serde(default)]
    pub tool_names: Vec<String>,

    /// First argv element after shell expansion (e.g. `git`, `cargo`).
    #[serde(default)]
    pub argv0: Vec<String>,

    /// For `git` specifically, the subcommand (`status`, `diff`, ...).
    #[serde(default)]
    pub git_subcommands: Vec<String>,

    /// Every inner array must have all its tokens present somewhere in argv.
    #[serde(default)]
    pub argv_includes: Vec<Vec<String>>,

    /// Any inner array whose tokens are all present qualifies.
    #[serde(default)]
    pub argv_includes_any: Vec<Vec<String>>,

    /// Substrings that must all appear in the raw command line.
    #[serde(default)]
    pub command_includes: Vec<String>,

    /// Any one of these substrings appearing in the raw command line qualifies.
    #[serde(default)]
    pub command_includes_any: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuleFilters {
    /// Regexes; matching lines are dropped.
    #[serde(default)]
    pub skip_patterns: Vec<String>,
    /// Regexes; only matching lines are kept (applied after skip).
    #[serde(default)]
    pub keep_patterns: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuleTransforms {
    #[serde(default)]
    pub strip_ansi: bool,
    #[serde(default)]
    pub pretty_print_json: bool,
    #[serde(default)]
    pub dedupe_adjacent: bool,
    #[serde(default)]
    pub trim_empty_edges: bool,
    /// Collapse runs of >=2 blank lines into a single blank line.
    #[serde(default)]
    pub fold_blank_runs: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuleSummarize {
    /// Number of leading lines to keep after filtering.
    #[serde(default)]
    pub head: Option<usize>,
    /// Number of trailing lines to keep after filtering.
    #[serde(default)]
    pub tail: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleCounter {
    pub name: String,
    pub pattern: String,
    #[serde(default)]
    pub flags: Option<String>,
}

/// A single rule loaded from JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonRule {
    /// Stable identifier. Later layers with the same `id` replace earlier ones.
    pub id: String,
    /// Logical grouping (e.g. `git`, `npm`). Surfaced in trace output.
    pub family: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Higher fires first when multiple rules match. Default 0.
    #[serde(default)]
    pub priority: i32,

    pub r#match: RuleMatch,

    #[serde(default)]
    pub filters: RuleFilters,
    #[serde(default)]
    pub transforms: RuleTransforms,
    #[serde(default)]
    pub summarize: Option<RuleSummarize>,
    #[serde(default)]
    pub counters: Vec<RuleCounter>,
}

impl JsonRule {
    /// Wrap as a layered rule with explicit origin metadata.
    pub fn into_layered(self, origin: RuleOrigin, source_path: Option<String>) -> LayeredRule {
        LayeredRule {
            rule: self,
            origin,
            source_path,
        }
    }
}

/// A rule plus the origin layer it came from.
#[derive(Debug, Clone)]
pub struct LayeredRule {
    pub rule: JsonRule,
    pub origin: RuleOrigin,
    pub source_path: Option<String>,
}

impl LayeredRule {
    pub(crate) fn precedence(&self) -> u8 {
        self.origin.precedence()
    }
}
