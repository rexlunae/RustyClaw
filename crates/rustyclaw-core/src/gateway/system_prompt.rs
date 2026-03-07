//! System prompt builder for TUI and messenger handlers.
//!
//! Provides a unified way to build system prompts with all necessary context
//! including workspace files (SOUL.md, etc.), skills, tasks, and guidelines.

use crate::config::Config;
use crate::workspace_context::{SessionType, WorkspaceContext};

use super::{SharedModelRegistry, SharedSkillManager, SharedTaskManager};

/// Session context for building system prompts.
pub struct SessionContext<'a> {
    /// Session type (Main for TUI/DM, Group for channels)
    pub session_type: SessionType,
    /// Optional session key for task tracking
    pub session_key: Option<&'a str>,
    /// Optional platform name (e.g., "discord", "telegram", "tui")
    pub platform: Option<&'a str>,
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
            channel: None,
            sender: None,
        }
    }
}

/// Build a complete system prompt for TUI or messenger sessions.
///
/// This function builds the full system prompt including:
/// - Base system prompt from config
/// - Safety guardrails
/// - Workspace context (SOUL.md, AGENTS.md, TOOLS.md, etc.)
/// - Skills context
/// - Active tasks section
/// - Model guidance
/// - Tool usage guidelines
/// - Silent reply and heartbeat guidance
/// - Runtime info
pub async fn build_system_prompt(
    config: &Config,
    task_mgr: &SharedTaskManager,
    model_registry: &SharedModelRegistry,
    skill_mgr: &SharedSkillManager,
    ctx: SessionContext<'_>,
) -> String {
    let base_prompt = config
        .system_prompt
        .clone()
        .unwrap_or_else(|| "You are a helpful AI assistant running inside RustyClaw.".to_string());

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

    // Add active tasks section if any
    if let Some(session_key) = ctx.session_key {
        if let Some(task_section) =
            super::task_handler::generate_task_prompt_section(task_mgr, session_key).await
        {
            parts.push(task_section);
        }
    }

    // Add model selection guidance for sub-agents
    let model_guidance = super::model_handler::generate_model_prompt_section(model_registry).await;
    parts.push(model_guidance);

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

    // Add context section if we have platform/channel/sender info
    if ctx.platform.is_some() || ctx.channel.is_some() || ctx.sender.is_some() {
        parts.push(format!(
            "## Session Context\n\
            - Platform: {}\n\
            - Channel: {}\n\
            - Sender: {}\n\
            \n\
            When responding:\n\
            - Be concise and helpful\n\
            - You have access to tools — use them when helpful",
            ctx.platform.unwrap_or("unknown"),
            ctx.channel.unwrap_or("direct"),
            ctx.sender.unwrap_or("user"),
        ));
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

### Sub-Agents
Spawn sub-agents for complex or time-consuming tasks:
- `sessions_spawn(task=\"...\", model=\"...\")` — runs asynchronously
- Results auto-announce when complete — no polling needed
- Use cheaper models for simple tasks (llama3.2, claude-haiku)

### Tool Call Style
- Default: don't narrate routine tool calls (just call them)
- Narrate only for: multi-step work, complex problems, sensitive actions
- Keep narration brief and value-dense
- Use plain language unless in technical context"
        .to_string()
}
