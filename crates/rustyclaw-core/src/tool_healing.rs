//! Self-healing for model tool calls (#462).
//!
//! Models damage their own tool calls in a handful of recurring ways:
//! truncated or otherwise malformed argument JSON, the same call emitted
//! twice in one response, tool-call markup leaking into the visible text,
//! and the same call repeated round after round. Each used to be handled by
//! failing — the arguments became `{}`, the duplicate ran twice, the markup
//! reached the transcript, the repeat burned a round — which taxes exactly
//! the models that need the most help.
//!
//! This module is the one place those repairs live. The hooks sit at the
//! provider choke points (`genai_backend`'s argument normalisation and
//! stream consumer, `providers::call_with_tools`) so every provider and
//! every caller — interactive, cron, messenger, subagent — inherits them.
//!
//! Everything here is behind `[tool_healing] enabled` (default on),
//! installed process-wide at gateway boot like `tool_limits`: the hooks run
//! deep in code that has no `Config` handle.

use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::gateway::protocol::types::ParsedToolCall;

// ── Config ──────────────────────────────────────────────────────────────────

fn default_true() -> bool {
    true
}

/// Whether the healing layer is active.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolHealingConfig {
    /// Master switch for every repair in this module.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for ToolHealingConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

fn config_cell() -> &'static Mutex<ToolHealingConfig> {
    static CONFIG: OnceLock<Mutex<ToolHealingConfig>> = OnceLock::new();
    CONFIG.get_or_init(|| Mutex::new(ToolHealingConfig::default()))
}

/// Replace the active config. Called once at gateway boot.
pub fn install(config: ToolHealingConfig) {
    if let Ok(mut slot) = config_cell().lock() {
        *slot = config;
    }
}

/// Whether healing is currently enabled.
///
/// A poisoned lock reports enabled — healing is repair, not policy, and the
/// default is on.
pub fn enabled() -> bool {
    config_cell().lock().map(|c| c.enabled).unwrap_or(true)
}

// ── JSON repair ─────────────────────────────────────────────────────────────

/// Try to recover a JSON object from a malformed argument string.
///
/// Handles, in order, the damage actually seen from models:
/// - arguments wrapped in a markdown code fence or surrounding prose
/// - trailing commas before `}` or `]`
/// - truncation — the string ends mid-value, mid-string, or with unclosed
///   braces (the classic max-tokens cut)
///
/// Returns `Some` only for a JSON **object**, because that is the only shape
/// tool arguments may take. `None` means unrepairable; the caller decides
/// what to do (today: fall back to `{}` exactly as before).
pub fn repair_json(input: &str) -> Option<serde_json::Value> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Cut prose and fences away: everything from the first `{` on. (The
    // closing side is handled by the truncation completer, so a missing
    // last `}` is fine.)
    let start = trimmed.find('{')?;
    let candidate = &trimmed[start..];

    // Fenced blocks end with ``` after the JSON; prose can trail too. Try
    // the widest object first — first `{` to last `}` — then fall through
    // to repairs on the full tail.
    if let Some(end) = candidate.rfind('}') {
        if let Ok(v @ serde_json::Value::Object(_)) =
            serde_json::from_str::<serde_json::Value>(&candidate[..=end])
        {
            return Some(v);
        }
    }

    let repaired = complete_truncated_json(&strip_trailing_commas(candidate));
    match serde_json::from_str::<serde_json::Value>(&repaired) {
        Ok(v @ serde_json::Value::Object(_)) => Some(v),
        _ => None,
    }
}

/// Remove trailing commas before a closing brace/bracket, outside strings.
fn strip_trailing_commas(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut escaped = false;
    for c in input.chars() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '}' | ']' => {
                // Drop a comma (and whitespace after it) that directly
                // precedes this closer.
                while let Some(last) = out.chars().last() {
                    if last.is_whitespace() {
                        out.pop();
                    } else {
                        break;
                    }
                }
                if out.ends_with(',') {
                    out.pop();
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Close whatever a truncated JSON string left open: an unterminated string,
/// then every unclosed brace and bracket, in nesting order.
///
/// A tail that ends mid-key or dangling (`"a": ` or `"a":`) is trimmed back
/// to the previous complete value first, since no closer can make it parse.
fn complete_truncated_json(input: &str) -> String {
    let mut stack: Vec<char> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for c in input.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' if stack.last() == Some(&c) => {
                stack.pop();
            }
            _ => {}
        }
    }

    let mut out = input.to_string();
    if in_string {
        // An escape split by the cut ("…\\") would swallow the closing
        // quote we are about to add.
        if escaped {
            out.pop();
        }
        out.push('"');
    }
    // `"key":` or `"key",` with nothing after it — no closer helps; trim the
    // dangling member so the object closes after the previous complete one.
    // Removing a `:` orphans the key string before it, which must go too, or
    // the object closes on a key with no value and still does not parse.
    loop {
        let t = out.trim_end();
        if t.ends_with(',') {
            out.truncate(t.len() - 1);
        } else if t.ends_with(':') {
            out.truncate(t.len() - 1);
            trim_trailing_string(&mut out);
        } else {
            break;
        }
    }
    for closer in stack.iter().rev() {
        out.push(*closer);
    }
    out
}

/// Remove a complete JSON string (`"…"`) from the end of `out`, if one is
/// there — used to drop the key a trimmed `:` left orphaned.
fn trim_trailing_string(out: &mut String) {
    let trimmed = out.trim_end();
    if !trimmed.ends_with('"') || trimmed.len() < 2 {
        return;
    }
    // Walk back to the opening quote, honouring escapes: a quote is the
    // opener only when preceded by an even run of backslashes.
    let bytes = trimmed.as_bytes();
    let mut i = trimmed.len() - 1; // the closing quote
    while i > 0 {
        i -= 1;
        if bytes[i] == b'"' {
            let mut backslashes = 0;
            while i > backslashes && bytes[i - 1 - backslashes] == b'\\' {
                backslashes += 1;
            }
            if backslashes % 2 == 0 {
                out.truncate(i);
                return;
            }
        }
    }
}

// ── Schema coercion ─────────────────────────────────────────────────────────

/// Coerce argument values toward the tool's declared parameter types.
///
/// Models pass `5` where a schema says string, or `"5"` where it says
/// integer; both then fail the tool's own validation as if the parameter
/// were missing. The declared type is known — the registry sends it to the
/// model in the first place — so an unambiguous conversion is applied
/// instead: number/bool → string when a string is declared, numeric string
/// → number, `"true"`/`"false"` → bool. Anything else is left alone; a
/// value that cannot be converted losslessly is the tool's problem to
/// report, not this layer's to guess at.
pub fn coerce_arguments(tool_name: &str, args: serde_json::Value) -> serde_json::Value {
    if !enabled() {
        return args;
    }
    let Some(props) = schema_properties(tool_name) else {
        return args;
    };
    coerce_with_properties(&props, args)
}

/// The declared `properties` map of a tool's parameter schema.
fn schema_properties(tool_name: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    crate::tools::tools_openai().into_iter().find_map(|def| {
        let function = def.get("function")?;
        if function.get("name")?.as_str()? != tool_name {
            return None;
        }
        function
            .get("parameters")?
            .get("properties")?
            .as_object()
            .cloned()
    })
}

fn coerce_with_properties(
    props: &serde_json::Map<String, serde_json::Value>,
    args: serde_json::Value,
) -> serde_json::Value {
    let serde_json::Value::Object(mut map) = args else {
        return args;
    };
    for (key, value) in map.iter_mut() {
        let Some(declared) = props
            .get(key)
            .and_then(|p| p.get("type"))
            .and_then(|t| t.as_str())
        else {
            continue;
        };
        let coerced = match (declared, &*value) {
            ("string", serde_json::Value::Number(n)) => {
                Some(serde_json::Value::String(n.to_string()))
            }
            ("string", serde_json::Value::Bool(b)) => {
                Some(serde_json::Value::String(b.to_string()))
            }
            ("integer", serde_json::Value::String(s)) => s
                .trim()
                .parse::<i64>()
                .ok()
                .map(|n| serde_json::Value::Number(n.into())),
            ("number", serde_json::Value::String(s)) => s
                .trim()
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(serde_json::Value::Number),
            ("boolean", serde_json::Value::String(s)) => match s.trim() {
                "true" => Some(serde_json::Value::Bool(true)),
                "false" => Some(serde_json::Value::Bool(false)),
                _ => None,
            },
            _ => None,
        };
        if let Some(new_value) = coerced {
            tracing::info!(
                parameter = %key,
                declared,
                "Healing: coerced a tool argument to its declared type"
            );
            *value = new_value;
        }
    }
    serde_json::Value::Object(map)
}

// ── Duplicate tool calls ────────────────────────────────────────────────────

/// Drop exact duplicates — same tool, same arguments — from one response.
///
/// A model that emits the same call twice in a single response means it
/// once: running both does the work twice (and for a mutating tool, does
/// the damage twice). Only *within-response* duplicates are dropped; a
/// repeat in a later round can be legitimate (re-reading a file after an
/// edit), so that case is only counted, by [`RepeatCallTracker`].
pub fn dedupe_tool_calls(calls: Vec<ParsedToolCall>) -> Vec<ParsedToolCall> {
    if !enabled() || calls.len() < 2 {
        return calls;
    }
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut kept = Vec::with_capacity(calls.len());
    let mut dropped = 0usize;
    for call in calls {
        let key = (call.name.clone(), call.arguments.to_string());
        if seen.contains(&key) {
            dropped += 1;
            tracing::info!(
                tool = %call.name,
                "Healing: dropped a duplicate tool call from one response"
            );
            continue;
        }
        seen.push(key);
        kept.push(call);
    }
    if dropped > 0 {
        tracing::info!(dropped, "Healing: response carried duplicate tool calls");
    }
    kept
}

// ── Repeated-call tracking ──────────────────────────────────────────────────

/// Counts consecutive identical calls (same tool, same arguments) across the
/// rounds of one turn.
///
/// The malformed-call suppressor already stops identical *failing* calls;
/// this covers the identical call that keeps *succeeding* — a model stuck
/// re-reading the same file expecting different content. That is warned
/// about, never blocked: polling loops (run tests, check a process) repeat
/// legitimately, and a wrong block is worse than a redundant round.
#[derive(Default)]
pub struct RepeatCallTracker {
    last: Option<(String, String)>,
    count: usize,
}

/// Consecutive identical calls at which [`RepeatCallTracker::observe`]
/// starts flagging.
pub const REPEAT_WARN_THRESHOLD: usize = 5;

impl RepeatCallTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a call; returns `Some(n)` — the run length — once the same
    /// call has been made [`REPEAT_WARN_THRESHOLD`] or more times in a row.
    pub fn observe(&mut self, name: &str, arguments: &serde_json::Value) -> Option<usize> {
        if !enabled() {
            return None;
        }
        let key = (name.to_string(), arguments.to_string());
        if self.last.as_ref() == Some(&key) {
            self.count += 1;
        } else {
            self.last = Some(key);
            self.count = 1;
        }
        (self.count >= REPEAT_WARN_THRESHOLD).then_some(self.count)
    }
}

/// The note appended to a tool result once a call repeats past the
/// threshold — addressed to the model, which is the party looping.
pub fn repeat_warning(name: &str, count: usize) -> String {
    format!(
        "\n\n[note: this exact `{name}` call has now been made {count} times in a row \
         with identical arguments. If the result is not changing, calling again will \
         not change it — adjust the arguments or take a different action.]"
    )
}

// ── Leaked tool markup ──────────────────────────────────────────────────────

/// Tool-call markup that some models emit into their *text* instead of the
/// provider's tool-call channel. Anything between a matched open/close pair
/// is a tool call that already failed to be one — it never parsed, it will
/// never run — so the block is removed from the visible text entirely.
const LEAK_TAGS: &[(&str, &str)] = &[
    ("<tool_call>", "</tool_call>"),
    ("<tool_calls>", "</tool_calls>"),
    ("<function_call>", "</function_call>"),
    ("<function_calls>", "</function_calls>"),
    ("<tool_use>", "</tool_use>"),
    ("<|tool_call|>", "<|/tool_call|>"),
];

/// Streaming filter that removes leaked tool markup from text chunks.
///
/// Chunks split tags arbitrarily, so the filter holds back the shortest
/// suffix that could still become a known open tag and emits everything
/// else immediately — worst-case latency is one tag's length of text, only
/// when the text ends in something tag-shaped. Inside a matched tag,
/// content is swallowed until the closing tag. [`Self::finish`] flushes:
/// a held prefix that never became a tag is emitted (it was ordinary
/// text); an unterminated leak block is dropped (it was markup).
#[derive(Default)]
pub struct StreamSanitizer {
    /// Trailing text held because it may be the start of an open tag.
    held: String,
    /// The close tag being awaited, once an open tag has matched.
    suppressing_until: Option<&'static str>,
    /// Swallowed content inside the current leak block (kept only so a
    /// close tag split across chunks can be matched).
    suppressed: String,
}

impl StreamSanitizer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one chunk; returns the text safe to emit now.
    pub fn feed(&mut self, chunk: &str) -> String {
        if !enabled() {
            return chunk.to_string();
        }
        let mut pending = std::mem::take(&mut self.held);
        pending.push_str(chunk);
        let mut out = String::with_capacity(pending.len());

        loop {
            if let Some(close) = self.suppressing_until {
                // Swallowing a leak block: look for its close tag.
                self.suppressed.push_str(&pending);
                if let Some(pos) = self.suppressed.find(close) {
                    let rest = self.suppressed[pos + close.len()..].to_string();
                    self.suppressed.clear();
                    self.suppressing_until = None;
                    pending = rest;
                    continue;
                }
                // Keep only the tail that could still hold a split close
                // tag; the front can never match again and would grow
                // without bound on a long block.
                let keep = close.len().saturating_sub(1);
                if self.suppressed.len() > keep {
                    let cut = self.suppressed.len() - keep;
                    // Cut on a char boundary — the tail only needs to be
                    // at least `keep` bytes, so walk back to a boundary.
                    let mut cut = cut;
                    while !self.suppressed.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    self.suppressed.drain(..cut);
                }
                return out;
            }

            // Not suppressing: find the earliest full open tag, if any.
            let earliest = LEAK_TAGS
                .iter()
                .filter_map(|(open, close)| pending.find(open).map(|p| (p, *open, *close)))
                .min_by_key(|(p, _, _)| *p);
            if let Some((pos, open, close)) = earliest {
                out.push_str(&pending[..pos]);
                let after = pending[pos + open.len()..].to_string();
                self.suppressing_until = Some(close);
                self.suppressed.clear();
                tracing::info!(
                    tag = open,
                    "Healing: stripping leaked tool markup from text"
                );
                pending = after;
                continue;
            }

            // No full tag. Hold back the longest suffix that is a prefix of
            // any open tag; emit the rest.
            let hold_from = longest_tag_prefix_suffix(&pending);
            out.push_str(&pending[..hold_from]);
            self.held = pending[hold_from..].to_string();
            return out;
        }
    }

    /// End of stream: release what was held, drop what was suppressed.
    pub fn finish(&mut self) -> String {
        self.suppressed.clear();
        self.suppressing_until = None;
        std::mem::take(&mut self.held)
    }
}

/// Byte offset from which the string's suffix is a proper prefix of some
/// known open tag (`s.len()` when none is).
fn longest_tag_prefix_suffix(s: &str) -> usize {
    // Tags are ASCII, so byte windows shorter than the tag are candidates.
    let max_len = LEAK_TAGS
        .iter()
        .map(|(open, _)| open.len() - 1)
        .max()
        .unwrap_or(0);
    let bytes = s.as_bytes();
    let window = max_len.min(bytes.len());
    for start in bytes.len() - window..bytes.len() {
        if !s.is_char_boundary(start) {
            continue;
        }
        let suffix = &s[start..];
        if LEAK_TAGS
            .iter()
            .any(|(open, _)| open.len() > suffix.len() && open.starts_with(suffix))
        {
            return start;
        }
    }
    s.len()
}

/// Remove leaked tool markup from a complete text (the batch counterpart of
/// [`StreamSanitizer`]).
pub fn strip_leaked_tool_markup(text: &str) -> String {
    let mut s = StreamSanitizer::new();
    let mut out = s.feed(text);
    out.push_str(&s.finish());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // The config is process-global and tests run in parallel; every test
    // here relies on the default (enabled), which install() is never called
    // to change in this suite.

    // ── repair_json ────────────────────────────────────────────────

    #[test]
    fn intact_json_passes_through() {
        assert_eq!(
            repair_json(r#"{"path": "a.txt"}"#),
            Some(json!({"path": "a.txt"}))
        );
    }

    #[test]
    fn a_markdown_fence_is_peeled() {
        let input = "```json\n{\"path\": \"a.txt\"}\n```";
        assert_eq!(repair_json(input), Some(json!({"path": "a.txt"})));
    }

    #[test]
    fn surrounding_prose_is_peeled() {
        let input = r#"Here are the arguments: {"path": "a.txt"} — let me know!"#;
        assert_eq!(repair_json(input), Some(json!({"path": "a.txt"})));
    }

    #[test]
    fn trailing_commas_are_removed() {
        assert_eq!(
            repair_json(r#"{"a": 1, "b": [1, 2,],}"#),
            Some(json!({"a": 1, "b": [1, 2]}))
        );
    }

    #[test]
    fn truncated_json_is_completed() {
        // The classic max-tokens cut: mid-string, braces unclosed.
        assert_eq!(
            repair_json(r#"{"path": "src/main.rs", "content": "fn main"#),
            Some(json!({"path": "src/main.rs", "content": "fn main"}))
        );
        // Cut right after a key's colon: the dangling member is dropped.
        assert_eq!(
            repair_json(r#"{"path": "a.txt", "content":"#),
            Some(json!({"path": "a.txt"}))
        );
        // Cut inside a nested structure.
        assert_eq!(
            repair_json(r#"{"a": {"b": [1, 2"#),
            Some(json!({"a": {"b": [1, 2]}}))
        );
    }

    #[test]
    fn a_cut_through_an_escape_still_closes_the_string() {
        // Ends with a lone backslash inside a string: naively appending a
        // quote would escape it and leave the string open.
        let repaired = repair_json(r#"{"content": "line\"#).expect("repairable");
        assert!(repaired.is_object());
    }

    #[test]
    fn hopeless_input_is_not_invented() {
        assert_eq!(repair_json(""), None);
        assert_eq!(repair_json("the model said nothing useful"), None);
        // A bare array is valid JSON but not valid *arguments*.
        assert_eq!(repair_json("[1, 2, 3]"), None);
    }

    // ── coerce_with_properties ─────────────────────────────────────

    fn props(spec: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        spec.as_object().cloned().expect("object literal")
    }

    #[test]
    fn declared_string_accepts_a_number_or_bool() {
        let p = props(json!({"line": {"type": "string"}, "flag": {"type": "string"}}));
        let out = coerce_with_properties(&p, json!({"line": 42, "flag": true}));
        assert_eq!(out, json!({"line": "42", "flag": "true"}));
    }

    #[test]
    fn declared_numbers_and_bools_accept_their_string_forms() {
        let p = props(json!({
            "count": {"type": "integer"},
            "ratio": {"type": "number"},
            "deep": {"type": "boolean"},
        }));
        let out =
            coerce_with_properties(&p, json!({"count": "7", "ratio": " 0.5 ", "deep": "true"}));
        assert_eq!(out, json!({"count": 7, "ratio": 0.5, "deep": true}));
    }

    #[test]
    fn unconvertible_values_are_left_for_the_tool_to_report() {
        let p = props(json!({"count": {"type": "integer"}}));
        let out = coerce_with_properties(&p, json!({"count": "several"}));
        assert_eq!(out, json!({"count": "several"}), "no guessing");
    }

    #[test]
    fn undeclared_parameters_are_untouched() {
        let p = props(json!({"path": {"type": "string"}}));
        let out = coerce_with_properties(&p, json!({"path": "a", "extra": 5}));
        assert_eq!(out, json!({"path": "a", "extra": 5}));
    }

    // ── dedupe_tool_calls ──────────────────────────────────────────

    fn call(id: &str, name: &str, args: serde_json::Value) -> ParsedToolCall {
        ParsedToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: args,
        }
    }

    #[test]
    fn exact_duplicates_in_one_response_collapse_to_one() {
        let calls = vec![
            call("1", "read_file", json!({"path": "a"})),
            call("2", "read_file", json!({"path": "a"})),
            call("3", "read_file", json!({"path": "b"})),
        ];
        let kept = dedupe_tool_calls(calls);
        assert_eq!(kept.len(), 2, "the exact duplicate must be dropped");
        assert_eq!(kept[0].id, "1", "the first occurrence is the one kept");
        assert_eq!(kept[1].arguments, json!({"path": "b"}));
    }

    #[test]
    fn same_tool_different_arguments_is_not_a_duplicate() {
        let calls = vec![
            call("1", "read_file", json!({"path": "a"})),
            call("2", "read_file", json!({"path": "b"})),
        ];
        assert_eq!(dedupe_tool_calls(calls).len(), 2);
    }

    // ── RepeatCallTracker ──────────────────────────────────────────

    #[test]
    fn a_repeated_call_is_flagged_at_the_threshold_not_before() {
        let mut t = RepeatCallTracker::new();
        let args = json!({"path": "a"});
        for _ in 0..REPEAT_WARN_THRESHOLD - 1 {
            assert_eq!(t.observe("read_file", &args), None);
        }
        assert_eq!(
            t.observe("read_file", &args),
            Some(REPEAT_WARN_THRESHOLD),
            "the threshold call must be flagged"
        );
        assert_eq!(
            t.observe("read_file", &args),
            Some(REPEAT_WARN_THRESHOLD + 1),
            "and it keeps being flagged while the run continues"
        );
    }

    #[test]
    fn a_different_call_resets_the_run() {
        let mut t = RepeatCallTracker::new();
        let args = json!({"path": "a"});
        for _ in 0..REPEAT_WARN_THRESHOLD - 1 {
            t.observe("read_file", &args);
        }
        assert_eq!(t.observe("read_file", &json!({"path": "b"})), None);
        assert_eq!(
            t.observe("read_file", &args),
            None,
            "the earlier run must not resume after an interruption"
        );
    }

    // ── StreamSanitizer / strip_leaked_tool_markup ─────────────────

    #[test]
    fn plain_text_is_untouched() {
        assert_eq!(
            strip_leaked_tool_markup("Reading the file now."),
            "Reading the file now."
        );
        // Ordinary angle brackets and XML-ish text survive.
        assert_eq!(
            strip_leaked_tool_markup("compare a < b and use <em>this</em>"),
            "compare a < b and use <em>this</em>"
        );
    }

    #[test]
    fn a_leaked_block_is_removed_wholesale() {
        let text = "I'll read it.<tool_call>{\"name\": \"read_file\"}</tool_call> Done.";
        assert_eq!(strip_leaked_tool_markup(text), "I'll read it. Done.");
    }

    #[test]
    fn a_tag_split_across_chunks_is_still_caught() {
        let mut s = StreamSanitizer::new();
        let mut out = String::new();
        for chunk in ["Sure. <tool_", "call>{\"a\": 1}</tool_", "call> Next."] {
            out.push_str(&s.feed(chunk));
        }
        out.push_str(&s.finish());
        assert_eq!(out, "Sure.  Next.");
    }

    #[test]
    fn held_text_that_never_becomes_a_tag_is_released() {
        let mut s = StreamSanitizer::new();
        let mut out = String::new();
        // "<tool" looks like a tag prefix but ends up being prose.
        out.push_str(&s.feed("the <tool"));
        out.push_str(&s.feed("box is heavy"));
        out.push_str(&s.finish());
        assert_eq!(out, "the <toolbox is heavy");
    }

    #[test]
    fn an_unterminated_leak_block_is_dropped_not_leaked() {
        let mut s = StreamSanitizer::new();
        let mut out = String::new();
        out.push_str(&s.feed("Okay. <tool_call>{\"name\": \"never_closed\""));
        out.push_str(&s.finish());
        assert_eq!(out, "Okay. ");
    }

    #[test]
    fn text_after_the_close_tag_in_the_same_chunk_survives() {
        let mut s = StreamSanitizer::new();
        let mut out = s.feed("a<tool_use>x</tool_use>b<tool_use>y</tool_use>c");
        out.push_str(&s.finish());
        assert_eq!(out, "abc");
    }

    #[test]
    fn multibyte_text_does_not_panic_the_holdback() {
        let mut s = StreamSanitizer::new();
        let mut out = String::new();
        out.push_str(&s.feed("émojis 🦀 and a trailing <"));
        out.push_str(&s.feed("tool_call>🦀</tool_call>🦀"));
        out.push_str(&s.finish());
        assert_eq!(out, "émojis 🦀 and a trailing 🦀");
    }
}
