//! Subagent profiles: narrowly-scoped agents spawned by a main agent.
//!
//! A subagent profile defines a *focused* agent: a short system prompt that
//! describes one job, and an explicit allowlist of the tools needed for that
//! job. The main agent spawns a subagent with `subagent_run`, feeding it the
//! task plus whatever context it needs. The subagent's model calls only ever
//! see the profile's tools, so it works without being distracted by the full
//! tool registry or the main agent's accumulated context.
//!
//! Profiles come from two places:
//! * **Built-ins** (code-writer, code-reviewer, bug-hunter, …) compiled into
//!   the binary — always available, never on disk.
//! * **Custom profiles** persisted as `<settings_dir>/subagents/<id>.toml`,
//!   created on demand (typically by the main agent via `subagent_create`).
//!
//! Like [`crate::agents::AgentRegistry`], the registry is filesystem-backed
//! with no in-memory cache so the gateway, CLI, and model tools always agree
//! on the profile list.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::agents::{is_valid_agent_id, sanitize_agent_id};

/// Default tool-loop rounds for a subagent run when the profile doesn't set
/// its own limit.
pub const DEFAULT_MAX_ROUNDS: usize = 24;

/// Hard ceiling on tool-loop rounds regardless of profile configuration.
pub const MAX_ROUNDS_CEILING: usize = 60;

/// A subagent profile: one focused job, one small toolset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentProfile {
    /// Stable id (lowercase, `-`/`_`), used with `subagent_run`.
    #[serde(default)]
    pub id: String,
    /// Display name.
    pub name: String,
    /// One-line description of what this subagent is for (shown to the main
    /// agent when it lists profiles).
    #[serde(default)]
    pub description: String,
    /// Focused system prompt. This *replaces* the main agent's system
    /// prompt — keep it tight and job-specific.
    pub system_prompt: String,
    /// Allowlist of tool names this subagent may see and use.
    pub tools: Vec<String>,
    /// Optional model override (must belong to the same provider as the
    /// main agent's active model).
    #[serde(default)]
    pub model: Option<String>,
    /// Optional tool-loop round limit for runs of this profile.
    #[serde(default)]
    pub max_rounds: Option<usize>,
    /// True for compiled-in profiles (never persisted).
    #[serde(skip)]
    pub builtin: bool,
}

impl SubagentProfile {
    /// Effective round limit for a run of this profile.
    pub fn effective_max_rounds(&self) -> usize {
        self.max_rounds
            .unwrap_or(DEFAULT_MAX_ROUNDS)
            .clamp(1, MAX_ROUNDS_CEILING)
    }
}

// ── Tool allowlist policy ───────────────────────────────────────────────────

/// Tool-name prefixes that can never appear in a subagent's toolset.
///
/// Subagents are meant to be *narrower* than the main agent: no spawning
/// further agents (recursion), no managing installation state (agents,
/// swarms, triggers, models, services, plugins), and no tool families that
/// are routed through gateway managers absent from the headless subagent
/// loop (secrets, skills, MCP, tasks).
const DENIED_PREFIXES: &[&str] = &[
    "subagent_",
    "sessions_",
    "agents_",
    "swarm_",
    "triggers_",
    "secrets_",
    "skill_",
    "mcp_",
    "model_",
    "task_",
    "service_",
    "plugin_",
];

/// Individual tools that can never appear in a subagent's toolset:
/// interactive prompts (no user is present), outward messaging (route
/// through the parent), destructive or installation-level operations, and
/// shared-state mutation (memory writes, todo list, thread metadata).
const DENIED_TOOLS: &[&str] = &[
    "ask_user",
    "client_dom_query",
    "session_status",
    "gateway",
    "message",
    "tts",
    "image",
    "image_generate",
    "canvas",
    "nodes",
    "cron",
    "agent_setup",
    "ollama_manage",
    "exo_manage",
    "pkg_manage",
    "net_scan",
    "service_manage",
    "user_manage",
    "firewall",
    "secure_delete",
    "save_memory",
    "add_memory",
    "todo",
    "thread_describe",
    "set_thread_caption",
    "screenshot",
    "clipboard",
];

/// Whether a tool may be included in a subagent profile's toolset.
pub fn tool_allowed_in_subagent(name: &str) -> bool {
    !DENIED_TOOLS.contains(&name) && !DENIED_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Validate a profile's tool allowlist: every entry must be a real tool
/// that is permitted inside subagents. Returns the deduplicated list.
pub fn validate_subagent_tools(tools: &[String]) -> Result<Vec<String>> {
    if tools.is_empty() {
        bail!("A subagent profile needs at least one tool");
    }
    let known = crate::tools::all_tool_names();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for tool in tools {
        let tool = tool.trim();
        if tool.is_empty() {
            continue;
        }
        if !known.contains(&tool) {
            bail!("Unknown tool '{}' in subagent toolset", tool);
        }
        if !tool_allowed_in_subagent(tool) {
            bail!(
                "Tool '{}' is not permitted in subagent toolsets (interactive, \
                 agent-management, and installation-level tools are reserved for \
                 the main agent)",
                tool
            );
        }
        if seen.insert(tool.to_string()) {
            out.push(tool.to_string());
        }
    }
    if out.is_empty() {
        bail!("A subagent profile needs at least one tool");
    }
    Ok(out)
}

// ── Built-in profiles ───────────────────────────────────────────────────────

/// The compiled-in subagent profiles. Ids here are reserved: custom
/// profiles cannot shadow them.
pub fn builtin_profiles() -> Vec<SubagentProfile> {
    vec![
        SubagentProfile {
            id: "code-writer".into(),
            name: "Code Writer".into(),
            description: "Implements a precisely-described code change and verifies it compiles"
                .into(),
            system_prompt: "\
You are a focused code-writing agent. You receive one well-scoped \
implementation task with the context you need. Your job is to make exactly \
that change — nothing more.

Method:
1. Read the files involved before editing; match the surrounding style.
2. Make the change with edit_file/write_file/apply_patch, or \
ast_grep_manage for structural rewrites.
3. Verify: run the project's build or test command with execute_command \
when one is apparent (e.g. cargo check, npm test). Fix what you broke.
4. Report: list every file you changed and what you did in each, plus the \
verification result. If you could not finish, say precisely what is missing.

Do not refactor unrelated code, do not add dependencies unless the task \
says to, and do not invent requirements beyond the task."
                .into(),
            tools: vec![
                "read_file".into(),
                "write_file".into(),
                "edit_file".into(),
                "apply_patch".into(),
                "list_directory".into(),
                "search_files".into(),
                "find_files".into(),
                "execute_command".into(),
                "process".into(),
                "ast_grep_manage".into(),
            ],
            model: None,
            max_rounds: None,
            builtin: true,
        },
        SubagentProfile {
            id: "code-reviewer".into(),
            name: "Code Reviewer".into(),
            description: "Reviews code for correctness, style, and safety without modifying it"
                .into(),
            system_prompt: "\
You are a focused code-review agent. You receive code (a diff, files, or a \
directory) and review it. You never modify files — you only read, search, \
and run read-only checks (linters, type checkers, tests) via \
execute_command.

Review for, in priority order:
1. Correctness — bugs, unhandled edge cases, race conditions, broken error \
handling.
2. Safety — injection, path traversal, secrets in code, unsafe operations.
3. Clarity — misleading names, dead code, needless complexity.
4. Consistency — deviations from the project's established style.

For every finding give: file:line, severity (critical/major/minor/nit), \
what is wrong, and a concrete fix. Verify a suspected bug by reading the \
callers/callees before reporting it. End with a short verdict summary. If \
the code is fine, say so — do not manufacture findings."
                .into(),
            tools: vec![
                "read_file".into(),
                "list_directory".into(),
                "search_files".into(),
                "find_files".into(),
                "execute_command".into(),
                "ast_grep_manage".into(),
            ],
            model: None,
            max_rounds: None,
            builtin: true,
        },
        SubagentProfile {
            id: "bug-hunter".into(),
            name: "Bug Hunter".into(),
            description: "Reproduces and root-causes a reported bug, without fixing it".into(),
            system_prompt: "\
You are a focused debugging agent. You receive a bug report or failure \
description with context. Your job is to find the root cause — not to fix \
it.

Method:
1. Reproduce: run the failing command/test with execute_command to see the \
real error.
2. Localize: follow the error to the code with search_files / \
ast_grep_manage / read_file. Read the actual code paths — do not guess.
3. Verify the hypothesis: add temporary diagnostics only if needed via \
execute_command (e.g. running with extra flags); never edit project files.
4. Report: the root cause (file:line and mechanism), the reproduction \
steps, the evidence that confirms it, and a suggested fix direction.

If you cannot reproduce the bug, report exactly what you tried and what \
you observed instead. Never claim a root cause you have not confirmed."
                .into(),
            tools: vec![
                "read_file".into(),
                "list_directory".into(),
                "search_files".into(),
                "find_files".into(),
                "execute_command".into(),
                "process".into(),
                "ast_grep_manage".into(),
            ],
            model: None,
            max_rounds: None,
            builtin: true,
        },
        SubagentProfile {
            id: "test-writer".into(),
            name: "Test Writer".into(),
            description: "Writes and runs tests for described behavior".into(),
            system_prompt: "\
You are a focused test-writing agent. You receive a description of code or \
behavior to cover. Your job is to write tests that would catch real \
regressions, following the project's existing test conventions.

Method:
1. Read the code under test and existing tests to learn the conventions \
(framework, naming, fixtures, file layout).
2. Write tests covering the main path, boundary cases, and error cases \
that matter. Prefer deterministic tests; avoid network and wall-clock \
dependence.
3. Run the tests with execute_command and make them pass against current \
behavior (or clearly flag a test that exposes an existing bug).
4. Report: the files you added/changed, what each test covers, and the \
test-run output.

Do not modify production code except when the task explicitly asks for \
testability hooks."
                .into(),
            tools: vec![
                "read_file".into(),
                "write_file".into(),
                "edit_file".into(),
                "apply_patch".into(),
                "list_directory".into(),
                "search_files".into(),
                "find_files".into(),
                "execute_command".into(),
                "process".into(),
                "ast_grep_manage".into(),
            ],
            model: None,
            max_rounds: None,
            builtin: true,
        },
        SubagentProfile {
            id: "researcher".into(),
            name: "Researcher".into(),
            description: "Researches a question using the web and local files, returns findings"
                .into(),
            system_prompt: "\
You are a focused research agent. You receive one question or topic with \
context. Your job is to find accurate, current information and distill it.

Method:
1. Search with web_search; fetch promising sources with web_fetch / \
web_extract. Consult local files with read_file when the task points at \
them.
2. Cross-check important claims across at least two independent sources.
3. Report: a direct answer first, then the key findings with the source \
URL for each, then open questions or caveats. Distinguish facts from \
inference. Say when sources conflict.

Stay on the assigned question; do not wander into adjacent topics."
                .into(),
            tools: vec![
                "web_search".into(),
                "web_fetch".into(),
                "web_extract".into(),
                "read_file".into(),
                "list_directory".into(),
                "search_files".into(),
                "find_files".into(),
            ],
            model: None,
            max_rounds: None,
            builtin: true,
        },
        SubagentProfile {
            id: "doc-writer".into(),
            name: "Doc Writer".into(),
            description: "Writes or updates documentation from the code as it actually is".into(),
            system_prompt: "\
You are a focused documentation agent. You receive a documentation task \
with context. Your job is to produce documentation that matches the code \
as it actually is — read the code first, never document from assumption.

Method:
1. Read the relevant code and any existing docs to match tone, format, and \
structure.
2. Write or update the documentation with write_file/edit_file. Keep \
examples runnable and minimal.
3. Verify every code reference (paths, names, flags, defaults) against the \
source before including it.
4. Report the files you touched and anything you found undocumented or \
contradictory that was out of scope.

Do not modify source code."
                .into(),
            tools: vec![
                "read_file".into(),
                "write_file".into(),
                "edit_file".into(),
                "list_directory".into(),
                "search_files".into(),
                "find_files".into(),
            ],
            model: None,
            max_rounds: None,
            builtin: true,
        },
    ]
}

// ── Registry ────────────────────────────────────────────────────────────────

/// Filesystem-backed subagent profile registry rooted at
/// `<settings_dir>/subagents`. Built-ins are always present; custom
/// profiles are one TOML file each.
#[derive(Debug, Clone)]
pub struct SubagentRegistry {
    profiles_root: PathBuf,
}

impl SubagentRegistry {
    /// Create a registry from the installation's settings dir.
    pub fn new(settings_dir: &Path) -> Self {
        Self {
            profiles_root: settings_dir.join("subagents"),
        }
    }

    /// Root directory holding one `<id>.toml` per custom profile.
    pub fn profiles_root(&self) -> &Path {
        &self.profiles_root
    }

    fn profile_path(&self, id: &str) -> PathBuf {
        self.profiles_root.join(format!("{}.toml", id))
    }

    /// Load one profile: built-ins first, then custom files. Invalid ids
    /// yield `None` (they could otherwise escape `profiles_root`).
    pub fn get(&self, id: &str) -> Option<SubagentProfile> {
        if let Some(p) = builtin_profiles().into_iter().find(|p| p.id == id) {
            return Some(p);
        }
        if !is_valid_agent_id(id) {
            return None;
        }
        let content = std::fs::read_to_string(self.profile_path(id)).ok()?;
        let mut profile: SubagentProfile = toml::from_str(&content).ok()?;
        profile.id = id.to_string();
        profile.builtin = false;
        Some(profile)
    }

    /// Whether a profile with this id exists (built-in or custom).
    pub fn exists(&self, id: &str) -> bool {
        builtin_profiles().iter().any(|p| p.id == id)
            || (is_valid_agent_id(id) && self.profile_path(id).exists())
    }

    /// List all profiles: built-ins first, then custom profiles sorted by id.
    pub fn list(&self) -> Vec<SubagentProfile> {
        let mut profiles = builtin_profiles();
        let mut custom_ids: Vec<String> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.profiles_root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                    continue;
                }
                let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                if is_valid_agent_id(id) && !profiles.iter().any(|p| p.id == id) {
                    custom_ids.push(id.to_string());
                }
            }
        }
        custom_ids.sort();
        profiles.extend(custom_ids.iter().filter_map(|id| self.get(id)));
        profiles
    }

    /// Create a custom profile. When `id` is `None` it is derived from the
    /// name. Tool names are validated against the registry and the subagent
    /// allowlist policy.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &self,
        id: Option<&str>,
        name: &str,
        description: &str,
        system_prompt: &str,
        tools: &[String],
        model: Option<&str>,
        max_rounds: Option<usize>,
    ) -> Result<SubagentProfile> {
        let name = name.trim();
        if name.is_empty() {
            bail!("Profile name must not be empty");
        }
        if system_prompt.trim().is_empty() {
            bail!("Profile system_prompt must not be empty");
        }
        let id = match id {
            Some(explicit) => {
                let sanitized = sanitize_agent_id(explicit);
                if sanitized.is_empty() || sanitized != explicit {
                    bail!(
                        "Invalid profile id '{}': use lowercase letters, digits, '-' or '_'",
                        explicit
                    );
                }
                sanitized
            }
            None => {
                let derived = sanitize_agent_id(name);
                if derived.is_empty() {
                    bail!("Could not derive a profile id from name '{}'", name);
                }
                derived
            }
        };
        if builtin_profiles().iter().any(|p| p.id == id) {
            bail!("'{}' is a built-in profile and cannot be replaced", id);
        }
        if self.exists(&id) {
            bail!("Subagent profile '{}' already exists", id);
        }
        let tools = validate_subagent_tools(tools)?;

        let profile = SubagentProfile {
            id: id.clone(),
            name: name.to_string(),
            description: description.trim().to_string(),
            system_prompt: system_prompt.to_string(),
            tools,
            model: model.map(str::to_string),
            max_rounds,
            builtin: false,
        };
        crate::persist::write_atomically(
            &self.profile_path(&id),
            toml::to_string_pretty(&profile)?.as_bytes(),
        )?;
        Ok(profile)
    }

    /// Delete a custom profile. Built-ins cannot be deleted.
    pub fn delete(&self, id: &str) -> Result<()> {
        if builtin_profiles().iter().any(|p| p.id == id) {
            bail!("'{}' is a built-in profile and cannot be deleted", id);
        }
        if !is_valid_agent_id(id) {
            bail!("Invalid profile id '{}'", id);
        }
        let path = self.profile_path(id);
        if !path.exists() {
            bail!("Subagent profile '{}' not found", id);
        }
        std::fs::remove_file(path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_tools() -> Vec<String> {
        vec!["read_file".into(), "search_files".into()]
    }

    #[test]
    fn builtins_are_valid() {
        let profiles = builtin_profiles();
        assert!(profiles.iter().any(|p| p.id == "code-writer"));
        assert!(profiles.iter().any(|p| p.id == "code-reviewer"));
        assert!(profiles.iter().any(|p| p.id == "bug-hunter"));
        for p in &profiles {
            assert!(p.builtin, "{} must be marked builtin", p.id);
            assert!(is_valid_agent_id(&p.id), "{} has an invalid id", p.id);
            assert!(!p.system_prompt.is_empty());
            // Every built-in toolset must pass the same validation applied
            // to custom profiles.
            let validated = validate_subagent_tools(&p.tools).unwrap();
            assert_eq!(validated, p.tools, "{} tools must be deduplicated", p.id);
        }
    }

    #[test]
    fn denied_tools_rejected() {
        for tool in [
            "ask_user",
            "sessions_spawn",
            "subagent_run",
            "secrets_get",
            "swarm_create",
            "agents_delete",
            "message",
            "save_memory",
        ] {
            assert!(!tool_allowed_in_subagent(tool), "{} must be denied", tool);
            let err = validate_subagent_tools(&[tool.to_string()]);
            assert!(err.is_err(), "{} must fail validation", tool);
        }
        assert!(tool_allowed_in_subagent("read_file"));
        assert!(tool_allowed_in_subagent("execute_command"));
        assert!(tool_allowed_in_subagent("web_search"));
    }

    #[test]
    fn unknown_tools_rejected() {
        assert!(validate_subagent_tools(&["definitely_not_a_tool".into()]).is_err());
        assert!(validate_subagent_tools(&[]).is_err());
    }

    #[test]
    fn create_get_list_delete_roundtrip() {
        let tmp = tempdir().unwrap();
        let reg = SubagentRegistry::new(tmp.path());

        let builtin_count = builtin_profiles().len();
        assert_eq!(reg.list().len(), builtin_count);

        let profile = reg
            .create(
                None,
                "License Checker",
                "checks dependency licenses",
                "You check licenses.",
                &sample_tools(),
                None,
                Some(10),
            )
            .unwrap();
        assert_eq!(profile.id, "license-checker");
        assert!(!profile.builtin);

        let listed = reg.list();
        assert_eq!(listed.len(), builtin_count + 1);
        let loaded = reg.get("license-checker").unwrap();
        assert_eq!(loaded.name, "License Checker");
        assert_eq!(loaded.tools, sample_tools());
        assert_eq!(loaded.max_rounds, Some(10));
        assert!(!loaded.builtin);

        reg.delete("license-checker").unwrap();
        assert_eq!(reg.list().len(), builtin_count);
        assert!(reg.get("license-checker").is_none());
    }

    #[test]
    fn builtin_ids_reserved() {
        let tmp = tempdir().unwrap();
        let reg = SubagentRegistry::new(tmp.path());
        assert!(
            reg.create(
                Some("code-writer"),
                "Fake",
                "",
                "prompt",
                &sample_tools(),
                None,
                None,
            )
            .is_err()
        );
        assert!(reg.delete("code-writer").is_err());
        // Built-ins are always gettable without any files on disk.
        assert!(reg.get("code-writer").is_some());
        assert!(reg.exists("bug-hunter"));
    }

    #[test]
    fn invalid_and_traversal_ids_rejected() {
        let tmp = tempdir().unwrap();
        // Decoy file outside the registry root that a traversal id could hit.
        let outside = tmp.path().join("victim.toml");
        std::fs::write(&outside, "name = \"victim\"\n").unwrap();

        let reg = SubagentRegistry::new(&tmp.path().join("state"));
        for id in ["../victim", "..", "a/../b", "Bad Id", ""] {
            assert!(!reg.exists(id), "id {:?} must not exist", id);
            assert!(reg.get(id).is_none(), "id {:?} must not load", id);
            assert!(reg.delete(id).is_err(), "id {:?} must not delete", id);
        }
        assert!(
            reg.create(
                Some("Bad Id"),
                "x",
                "",
                "prompt",
                &sample_tools(),
                None,
                None
            )
            .is_err()
        );
        assert!(outside.exists());
    }

    #[test]
    fn create_rejects_denied_and_unknown_tools() {
        let tmp = tempdir().unwrap();
        let reg = SubagentRegistry::new(tmp.path());
        assert!(
            reg.create(
                Some("sneaky"),
                "Sneaky",
                "",
                "prompt",
                &["read_file".into(), "secrets_get".into()],
                None,
                None,
            )
            .is_err()
        );
        assert!(
            reg.create(
                Some("typo"),
                "Typo",
                "",
                "prompt",
                &["raed_file".into()],
                None,
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn effective_rounds_clamped() {
        let mut p = builtin_profiles().remove(0);
        assert_eq!(p.effective_max_rounds(), DEFAULT_MAX_ROUNDS);
        p.max_rounds = Some(0);
        assert_eq!(p.effective_max_rounds(), 1);
        p.max_rounds = Some(10_000);
        assert_eq!(p.effective_max_rounds(), MAX_ROUNDS_CEILING);
    }
}
