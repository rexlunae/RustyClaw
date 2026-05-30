//! Pick the best matching rule for a `ToolInput`.

use crate::ToolInput;
use crate::compile::CompiledRule;
use crate::rule::RuleMatch;

/// Result of looking up a rule for the given input.
#[derive(Debug)]
pub struct Classification<'a> {
    pub rule: &'a CompiledRule,
    pub family: String,
}

/// Highest-priority matching rule, or `None`.
pub fn classify<'a>(
    rules: &'a [CompiledRule],
    input: &ToolInput<'_>,
) -> Option<Classification<'a>> {
    let mut best: Option<&CompiledRule> = None;
    for r in rules {
        if !matches(&r.layered.rule.r#match, input) {
            continue;
        }
        match best {
            None => best = Some(r),
            Some(b) => {
                if r.layered.rule.priority > b.layered.rule.priority {
                    best = Some(r);
                } else if r.layered.rule.priority == b.layered.rule.priority
                    && r.layered.precedence() > b.layered.precedence()
                {
                    // Tie-break: deeper layer wins.
                    best = Some(r);
                }
            }
        }
    }
    best.map(|r| Classification {
        rule: r,
        family: r.layered.rule.family.clone(),
    })
}

fn matches(m: &RuleMatch, input: &ToolInput<'_>) -> bool {
    if !m.tool_names.is_empty() && !m.tool_names.iter().any(|t| t == input.tool_name) {
        return false;
    }

    if !m.argv0.is_empty() {
        let Some(a0) = input.argv.first() else {
            return false;
        };
        if !m.argv0.iter().any(|t| t == a0) {
            return false;
        }
    }

    if !m.git_subcommands.is_empty() {
        let Some(a0) = input.argv.first() else {
            return false;
        };
        if a0 != "git" {
            return false;
        }
        let Some(sub) = input.argv.get(1) else {
            return false;
        };
        if !m.git_subcommands.iter().any(|t| t == sub) {
            return false;
        }
    }

    if !m.argv_includes.is_empty() {
        let all_match = m
            .argv_includes
            .iter()
            .all(|group| group.iter().all(|tok| input.argv.iter().any(|a| a == tok)));
        if !all_match {
            return false;
        }
    }

    if !m.argv_includes_any.is_empty() {
        let any_match = m
            .argv_includes_any
            .iter()
            .any(|group| group.iter().all(|tok| input.argv.iter().any(|a| a == tok)));
        if !any_match {
            return false;
        }
    }

    if !m.command_includes.is_empty() {
        let Some(cmd) = input.command else {
            return false;
        };
        if !m.command_includes.iter().all(|s| cmd.contains(s)) {
            return false;
        }
    }

    if !m.command_includes_any.is_empty() {
        let Some(cmd) = input.command else {
            return false;
        };
        if !m.command_includes_any.iter().any(|s| cmd.contains(s)) {
            return false;
        }
    }

    true
}
