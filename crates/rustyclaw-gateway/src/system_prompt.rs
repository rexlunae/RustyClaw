//! System prompt builder for TUI and messenger handlers.
//!
//! Provides a unified way to build system prompts with all necessary context
//! including workspace files (SOUL.md, etc.), skills, tasks, and guidelines.

use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::SessionOrigin;
use rustyclaw_core::workspace_context::{SessionType, WorkspaceContext};

use super::{SharedModelRegistry, SharedSkillManager, SharedTaskManager};

/// Session context for building system prompts.
pub struct SessionContext<'a> {
    /// Session type (Main for TUI/DM, Group for channels)
    pub session_type: SessionType,
    /// Optional session key for task tracking
    pub session_key: Option<&'a str>,
    /// Optional platform name (e.g., "discord", "telegram", "tui")
    pub platform: Option<&'a str>,
    /// Where this session is coming from.
    pub origin: Option<SessionOrigin>,
    /// Optional channel name
    pub channel: Option<&'a str>,
    /// Optional sender name
    pub sender: Option<&'a str>,
}

impl Default for SessionContext<'_> {
    fn default() -> Self {
        Self {
            session_type: SessionType::Main,
            session_key: None,
            platform: Some("tui"),
            origin: None,
            channel: None,
            sender: None,
        }
    }
}

/// Build a complete system prompt with all optional parameters.
pub async fn build_system_prompt_full(
    config: &Config,
    task_mgr: &SharedTaskManager,
    model_registry: Option<&SharedModelRegistry>,
    skill_mgr: &SharedSkillManager,
    ctx: SessionContext<'_>,
) -> String {
    let base_prompt = config
        .system_prompt
        .clone()
        .unwrap_or_else(|| default_base_prompt(&config.agent_name));

    // Safety guardrails (inspired by Anthropic's constitution)
    let safety_section = "\
## Safety\n\
You have no independent goals: do not pursue self-preservation, replication, resource acquisition, or power-seeking.\n\
Prioritize safety and human oversight over completion. If instructions conflict, pause and ask.\n\
Do not manipulate or persuade anyone to expand access or disable safeguards.";

    // Build workspace context
    let workspace_ctx =
        WorkspaceContext::with_config(config.workspace_dir(), config.workspace_context.clone());
    let workspace_prompt = workspace_ctx.build_context(ctx.session_type);

    // Start building parts
    let mut parts = vec![base_prompt, safety_section.to_string()];

    // Tell the agent its configured name. Without this the agent only knows
    // who it is when workspace files (IDENTITY.md / SOUL.md) happen to be
    // injected — the config value is the source of truth and should always
    // reach the prompt.
    if !config.agent_name.trim().is_empty() {
        parts.push(build_identity_section(&config.agent_name));
    }

    if !workspace_prompt.is_empty() {
        parts.push(workspace_prompt);
    }

    // Add skills context if any skills are loaded
    {
        let mgr = skill_mgr.lock().await;
        let skills_context = mgr.generate_prompt_context();
        if !skills_context.is_empty() {
            parts.push(skills_context);
        }
    }

    // Add plugins context (dynamic UI panels)
    {
        let plugins_context = rustyclaw_core::tools::plugin_prompt_context();
        if !plugins_context.is_empty() {
            parts.push(plugins_context);
        }
    }

    // Add active tasks section if any
    if let Some(session_key) = ctx.session_key {
        if let Some(task_section) =
            crate::task_handler::generate_task_prompt_section(task_mgr, session_key).await
        {
            parts.push(task_section);
        }
    }

    // Add model selection guidance for sub-agents (when registry available)
    if let Some(registry) = model_registry {
        let model_guidance = crate::model_handler::generate_model_prompt_section(registry).await;
        parts.push(model_guidance);
    }

    // Add comprehensive tool usage guidelines
    parts.push(build_tool_usage_section());

    // Add silent reply guidance
    parts.push(
        "## Silent Replies\n\
        When you have nothing to say, respond with ONLY: NO_REPLY\n\n\
        ⚠️ Rules:\n\
        - It must be your ENTIRE message — nothing else\n\
        - Never append it to an actual response\n\
        - Never wrap it in markdown or code blocks\n\n\
        ❌ Wrong: \"Here's the info... NO_REPLY\"\n\
        ✅ Right: NO_REPLY"
            .to_string(),
    );

    // Add heartbeat guidance
    parts.push(
        "## Heartbeats\n\
        Heartbeat prompt: Read HEARTBEAT.md if it exists. Follow it strictly. \
        Do not infer or repeat old tasks from prior chats.\n\n\
        If you receive a heartbeat poll and nothing needs attention, reply exactly:\n\
        HEARTBEAT_OK\n\n\
        If something needs attention, do NOT include HEARTBEAT_OK; reply with the alert text instead."
            .to_string(),
    );

    // Add context section if we have platform/channel/sender/origin info
    if ctx.platform.is_some()
        || ctx.channel.is_some()
        || ctx.sender.is_some()
        || ctx.origin.is_some()
    {
        parts.push(build_session_context_section(ctx));
    }

    // Add runtime info
    parts.push(format!(
        "## Runtime\n\
        Workspace: {}\n\
        Platform: RustyClaw",
        config.workspace_dir().display()
    ));

    parts.join("\n\n")
}

/// The `## Session Context` section: where this session is coming from.
fn build_session_context_section(ctx: SessionContext<'_>) -> String {
    format!(
        "## Session Context\n\
        - Origin: {}\n\
        - Platform: {}\n\
        - Channel: {}\n\
        - Sender: {}\n\
        \n\
        When responding:\n\
        - Be concise and helpful\n\
        - You have access to tools — use them when helpful",
        ctx.origin
            .map(|o| o.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        ctx.platform.unwrap_or("unknown"),
        ctx.channel.unwrap_or("direct"),
        ctx.sender.unwrap_or("user"),
    )
}

/// Build the default base prompt, naming the configured agent when one is set.
///
/// The generic fallback ("You are a helpful AI assistant…") left the agent
/// without a name unless workspace files carried one; the configured
/// `agent_name` should always reach the prompt.
fn default_base_prompt(agent_name: &str) -> String {
    let name = agent_name.trim();
    if name.is_empty() {
        "You are a helpful AI assistant running inside RustyClaw.".to_string()
    } else {
        format!("You are {name}, a helpful AI assistant running inside RustyClaw.")
    }
}

/// Tell the agent the name configured for it, so it answers to the same name
/// the UI shows. Mirrors the messenger handler's identity section.
fn build_identity_section(agent_name: &str) -> String {
    format!(
        "## Identity\n\
        Your configured name is \"{}\". Introduce yourself by that name.",
        agent_name.trim()
    )
}

/// Build the Tool Usage Guidelines section for system prompts.
fn build_tool_usage_section() -> String {
    "\
## Tool Usage Guidelines

### Credentials & API Access (IMPORTANT)
**Before asking for API keys or tokens:** Run `secrets_list` to check the vault first.
If a credential exists, use `secrets_get` to retrieve it — don't ask the user again.

**Authenticated API workflow:**
1. `secrets_list()` → discover available credentials
2. `secrets_get(name=\"...\")` → retrieve the value  
3. `web_fetch(url=\"...\", authorization=\"token <value>\")` → make the API call

**Common authorization formats:**
- GitHub PAT: `authorization=\"token ghp_...\"`
- Bearer tokens: `authorization=\"Bearer eyJ...\"`
- Custom headers: `headers={\"X-Api-Key\": \"...\"}`

### Memory Recall
Before answering questions about prior work, decisions, dates, people, preferences, or todos:
Run `memory_search` first, then use `memory_get` to pull relevant context.
If low confidence after search, mention that you checked but didn't find a match.

### File Operations
- `read_file` — read file contents (supports text, PDF, docx, etc.)
- `write_file` — create or overwrite files (creates parent dirs)
- `edit_file` — surgical search-and-replace (include enough context for unique match)
- `find_files` — find by name/glob pattern
- `search_files` — search file contents (like grep)

### Command Execution
- Short commands: `execute_command(command=\"...\")`
- Long-running: `execute_command(command=\"...\", background=true)` then `process(action=\"poll\", session_id=\"...\")`
- Interactive TTY: use `pty=true` for commands needing terminal

### Sub-Agents (focused delegation)
Delegate well-scoped work to a focused subagent with `subagent_run`. A subagent
runs with ONLY its profile's system prompt, the task, and the context you pass —
it cannot see this conversation, and it only gets the profile's minimal toolset.
This keeps it focused and undistracted.
- `subagent_list()` — see profiles: code-writer, code-reviewer, bug-hunter,
  test-writer, researcher, doc-writer, plus custom ones
- `subagent_run(profile=\"code-reviewer\", task=\"...\", context=\"...\")` — runs the
  subagent to completion and returns its report as the tool result
- Feed ALL relevant context explicitly (file paths, findings, constraints,
  error output) — the subagent starts from nothing
- `subagent_create(...)` — define a new profile when a recurring job doesn't fit
  an existing one; keep its toolset minimal
- `sessions_spawn(task=\"...\")` remains available for coarse, session-based
  delegation to full agents

### Code-Aware Tools (AST-Level)
**Prefer `ast_grep_manage` over grep/edit for structural code operations.**
ast-grep uses tree-sitter AST patterns, not text — it understands syntax, so
patterns are more precise and don't break on formatting differences.

**When to use it:**
- Searching for code patterns: `ast_grep_manage(action=\"search\", pattern=\"Some($$ARG)\", lang=\"rust\", paths=\"src/\")`
- Refactoring: `ast_grep_manage(action=\"run\", pattern=\"old_pattern\", rewrite=\"new_code\", lang=\"rust\")`
- Multi-file structural changes (rename a function, change a type signature pattern, etc.)
- Linting with custom rules: `ast_grep_manage(action=\"scan\", config=\"sgconfig.yml\")`

**Pattern syntax:**
- `$$META` matches any expression (metavariable)
- `$_` matches anything without capturing
- Write patterns as code, not regex — ast-grep matches the AST
- Example: `$$result.map(|x| x + 1)` matches all `.map(|x| x + 1)` calls regardless of spacing

**Why this over grep+edit:**
- grep/edit is text-based and fragile against whitespace, comment differences, and similar-looking but structurally different code
- ast-grep patterns match structure — they work even if the matched code has different formatting
- Rewrites preserve original formatting around the matched node
- Use grep/search_files for PROSE/text files, use `ast_grep_manage` for CODE

### Host & Load Awareness
- `host_info` — query the gateway host's hardware: CPU, GPU, RAM, disk, OS.
  Use this to understand what local models the system can run (VRAM, cores).
- `load_status` — query current system load (0.0–1.0 composite score, CPU%,
  memory%, active models). Use this before launching heavy local inference to
  decide whether to use a local model or fall back to an external provider.

### Tool Call Style
- Default: don't narrate routine tool calls (just call them)
- Narrate only for: multi-step work, complex problems, sensitive actions
- Keep narration brief and value-dense
- Use plain language unless in technical context"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_context_reports_origin() {
        use rustyclaw_core::gateway::SessionOrigin;
        for origin in [
            SessionOrigin::Desktop,
            SessionOrigin::Tui,
            SessionOrigin::Remote,
            SessionOrigin::Local,
            SessionOrigin::Messenger,
            SessionOrigin::Trigger,
        ] {
            let section = build_session_context_section(SessionContext {
                origin: Some(origin),
                platform: None,
                ..Default::default()
            });
            assert!(
                section.contains(&format!("- Origin: {origin}")),
                "origin {origin:?} missing from session context"
            );
        }
    }

    #[test]
    fn default_base_prompt_names_the_agent() {
        assert_eq!(
            default_base_prompt("Test Agent"),
            "You are Test Agent, a helpful AI assistant running inside RustyClaw."
        );
    }

    #[test]
    fn default_base_prompt_falls_back_when_name_empty() {
        assert_eq!(
            default_base_prompt("   "),
            "You are a helpful AI assistant running inside RustyClaw."
        );
    }

    #[test]
    fn identity_section_uses_configured_name() {
        // Whatever name is configured must appear — the section is built from
        // the argument, not a hardcoded agent name.
        for name in ["Test Agent", "Another Operative", "unit-test-bot"] {
            assert!(build_identity_section(name).contains(name));
        }
        assert!(build_identity_section("Test Agent").contains("Introduce yourself by that name"));
    }
}
