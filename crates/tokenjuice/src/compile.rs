//! Compile `JsonRule` patterns into regexes once, at load time.

use crate::rule::{LayeredRule, RuleCounter};
use regex::{Regex, RegexBuilder};

#[derive(Debug)]
pub struct CompiledCounter {
    pub name: String,
    pub pattern: Regex,
}

#[derive(Debug)]
pub struct CompiledRule {
    pub layered: LayeredRule,
    pub skip_patterns: Vec<Regex>,
    pub keep_patterns: Vec<Regex>,
    pub counters: Vec<CompiledCounter>,
}

impl CompiledRule {
    pub fn compile(layered: LayeredRule) -> Result<Self, CompileError> {
        let skip_patterns = layered
            .rule
            .filters
            .skip_patterns
            .iter()
            .map(|p| build_regex(p, None))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CompileError::with_rule(&layered.rule.id, e))?;

        let keep_patterns = layered
            .rule
            .filters
            .keep_patterns
            .iter()
            .map(|p| build_regex(p, None))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CompileError::with_rule(&layered.rule.id, e))?;

        let counters = layered
            .rule
            .counters
            .iter()
            .map(compile_counter)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CompileError::with_rule(&layered.rule.id, e))?;

        Ok(Self {
            layered,
            skip_patterns,
            keep_patterns,
            counters,
        })
    }
}

fn compile_counter(c: &RuleCounter) -> Result<CompiledCounter, String> {
    let regex = build_regex(&c.pattern, c.flags.as_deref())?;
    Ok(CompiledCounter {
        name: c.name.clone(),
        pattern: regex,
    })
}

fn build_regex(pattern: &str, flags: Option<&str>) -> Result<Regex, String> {
    let mut builder = RegexBuilder::new(pattern);
    if let Some(flags) = flags {
        for ch in flags.chars() {
            match ch {
                'i' => {
                    builder.case_insensitive(true);
                }
                'm' => {
                    builder.multi_line(true);
                }
                's' => {
                    builder.dot_matches_new_line(true);
                }
                'u' => {
                    builder.unicode(true);
                }
                'x' => {
                    builder.ignore_whitespace(true);
                }
                // JS `g` flag is meaningless for the `regex` crate (every
                // search is global) — silently accept it for compatibility.
                'g' => {}
                _ => return Err(format!("unsupported regex flag '{}'", ch)),
            }
        }
    }
    builder
        .build()
        .map_err(|e| format!("invalid regex /{}/: {}", pattern, e))
}

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("rule {id}: {msg}")]
    Rule { id: String, msg: String },
}

impl CompileError {
    fn with_rule(id: &str, msg: String) -> Self {
        Self::Rule {
            id: id.to_string(),
            msg,
        }
    }
}
