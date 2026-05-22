//! Default rule set shipped with the crate. Covers high-volume sources of
//! tool-output noise. Each rule is small and conservative — when in doubt,
//! a user-layer rule should override.

use crate::rule::{
    JsonRule, RuleCounter, RuleFilters, RuleMatch, RuleSummarize, RuleTransforms,
};

pub fn builtin_rules() -> Vec<JsonRule> {
    vec![
        git_status(),
        git_diff(),
        cargo_build(),
        npm_install(),
        docker_ps(),
        ls_long(),
        web_fetch_html(),
        generic_long_output(),
    ]
}

fn git_status() -> JsonRule {
    JsonRule {
        id: "tokenjuice.builtin.git.status".into(),
        family: "git".into(),
        description: Some("Collapse git status to changed-file summary.".into()),
        priority: 100,
        r#match: RuleMatch {
            git_subcommands: vec!["status".into()],
            ..Default::default()
        },
        filters: RuleFilters {
            skip_patterns: vec![
                r"^\s*\(use ".into(),
                r"^\s*\(.*to unstage\)".into(),
                r"^\s*\(.*to discard\)".into(),
                r"^\s*\(commit or discard\)".into(),
                r"^On branch ".into(),
                r"^Your branch is up to date".into(),
            ],
            keep_patterns: vec![],
        },
        transforms: RuleTransforms {
            strip_ansi: true,
            trim_empty_edges: true,
            fold_blank_runs: true,
            ..Default::default()
        },
        summarize: None,
        counters: vec![
            RuleCounter {
                name: "modified".into(),
                pattern: r"^\s*modified:".into(),
                flags: None,
            },
            RuleCounter {
                name: "untracked".into(),
                pattern: r"^\?\? ".into(),
                flags: None,
            },
        ],
    }
}

fn git_diff() -> JsonRule {
    JsonRule {
        id: "tokenjuice.builtin.git.diff".into(),
        family: "git".into(),
        description: Some("Keep head + tail of long diffs.".into()),
        priority: 90,
        r#match: RuleMatch {
            git_subcommands: vec!["diff".into()],
            ..Default::default()
        },
        filters: RuleFilters::default(),
        transforms: RuleTransforms {
            strip_ansi: true,
            ..Default::default()
        },
        summarize: Some(RuleSummarize {
            head: Some(200),
            tail: Some(100),
        }),
        counters: vec![],
    }
}

fn cargo_build() -> JsonRule {
    JsonRule {
        id: "tokenjuice.builtin.cargo.build".into(),
        family: "cargo".into(),
        description: Some("Keep warnings and the final result line; drop progress.".into()),
        priority: 80,
        r#match: RuleMatch {
            argv0: vec!["cargo".into()],
            argv_includes_any: vec![
                vec!["build".into()],
                vec!["check".into()],
                vec!["test".into()],
            ],
            ..Default::default()
        },
        filters: RuleFilters {
            skip_patterns: vec![
                r"^\s*Compiling ".into(),
                r"^\s*Downloading ".into(),
                r"^\s*Fetching ".into(),
                r"^\s*Updating ".into(),
                r"^\s*Checking ".into(),
                r"^\s*Building ".into(),
            ],
            keep_patterns: vec![],
        },
        transforms: RuleTransforms {
            strip_ansi: true,
            trim_empty_edges: true,
            fold_blank_runs: true,
            ..Default::default()
        },
        summarize: None,
        counters: vec![
            RuleCounter {
                name: "warnings".into(),
                pattern: r"^warning:".into(),
                flags: None,
            },
            RuleCounter {
                name: "errors".into(),
                pattern: r"^error".into(),
                flags: None,
            },
        ],
    }
}

fn npm_install() -> JsonRule {
    JsonRule {
        id: "tokenjuice.builtin.npm.install".into(),
        family: "npm".into(),
        description: Some("Drop progress, keep audit + final summary.".into()),
        priority: 70,
        r#match: RuleMatch {
            argv0: vec!["npm".into(), "pnpm".into()],
            argv_includes_any: vec![vec!["install".into()], vec!["i".into()], vec!["add".into()]],
            ..Default::default()
        },
        filters: RuleFilters {
            skip_patterns: vec![
                r"^npm warn deprecated".into(),
                r"^\s*\[".into(),
                r"^\s*⠋".into(),
                r"^\s*⠙".into(),
                r"^\s*⠹".into(),
                r"^\s*⠸".into(),
                r"^\s*⠼".into(),
                r"^\s*⠴".into(),
                r"^\s*⠦".into(),
                r"^\s*⠧".into(),
                r"^\s*⠇".into(),
                r"^\s*⠏".into(),
            ],
            keep_patterns: vec![],
        },
        transforms: RuleTransforms {
            strip_ansi: true,
            trim_empty_edges: true,
            fold_blank_runs: true,
            ..Default::default()
        },
        summarize: None,
        counters: vec![],
    }
}

fn docker_ps() -> JsonRule {
    JsonRule {
        id: "tokenjuice.builtin.docker.ps".into(),
        family: "docker".into(),
        description: Some("Keep header + running containers.".into()),
        priority: 60,
        r#match: RuleMatch {
            argv0: vec!["docker".into()],
            argv_includes_any: vec![vec!["ps".into()]],
            ..Default::default()
        },
        filters: RuleFilters::default(),
        transforms: RuleTransforms {
            strip_ansi: true,
            ..Default::default()
        },
        summarize: Some(RuleSummarize {
            head: Some(50),
            tail: Some(0),
        }),
        counters: vec![],
    }
}

fn ls_long() -> JsonRule {
    JsonRule {
        id: "tokenjuice.builtin.ls.long".into(),
        family: "filesystem".into(),
        description: Some("Truncate very long directory listings.".into()),
        priority: 50,
        r#match: RuleMatch {
            argv0: vec!["ls".into()],
            ..Default::default()
        },
        filters: RuleFilters::default(),
        transforms: RuleTransforms {
            strip_ansi: true,
            ..Default::default()
        },
        summarize: Some(RuleSummarize {
            head: Some(200),
            tail: Some(20),
        }),
        counters: vec![],
    }
}

fn web_fetch_html() -> JsonRule {
    JsonRule {
        id: "tokenjuice.builtin.web_fetch".into(),
        family: "web".into(),
        description: Some("Trim huge web-fetch results that already passed through html2md.".into()),
        priority: 40,
        r#match: RuleMatch {
            tool_names: vec!["web_fetch".into()],
            ..Default::default()
        },
        filters: RuleFilters {
            skip_patterns: vec![r"^\s*\!\[".into()],
            keep_patterns: vec![],
        },
        transforms: RuleTransforms {
            trim_empty_edges: true,
            fold_blank_runs: true,
            ..Default::default()
        },
        summarize: Some(RuleSummarize {
            head: Some(500),
            tail: Some(100),
        }),
        counters: vec![],
    }
}

/// Last-resort fallback for any tool that produces a very long output.
fn generic_long_output() -> JsonRule {
    JsonRule {
        id: "tokenjuice.builtin.generic.long".into(),
        family: "generic".into(),
        description: Some("Catch-all: head/tail summarize anything over ~10k lines.".into()),
        priority: -100,
        r#match: RuleMatch::default(),
        filters: RuleFilters::default(),
        transforms: RuleTransforms {
            trim_empty_edges: true,
            ..Default::default()
        },
        summarize: Some(RuleSummarize {
            head: Some(500),
            tail: Some(100),
        }),
        counters: vec![],
    }
}
