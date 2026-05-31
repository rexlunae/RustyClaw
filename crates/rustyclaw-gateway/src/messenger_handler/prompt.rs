//! Messenger system-prompt assembly.
//!
//! Builds the per-message system prompt: base persona, safety guardrails,
//! workspace context, skills, active tasks, model guidance, tool-usage
//! guidelines, and platform-specific formatting hints.

use rustyclaw_core::config::Config;
use rustyclaw_core::messengers::Message;

use crate::SharedSkillManager;

/// Build system prompt with messenger context, workspace files, active tasks, and model guidance.
pub(crate) async fn build_messenger_system_prompt(
    config: &Config,
    messenger_type: &str,
    msg: &Message,
    task_mgr: &crate::SharedTaskManager,
    model_registry: &crate::SharedModelRegistry,
    skill_mgr: &SharedSkillManager,
    session_key: &str,
) -> String {
    use rustyclaw_core::workspace_context::{SessionType, WorkspaceContext};

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

    // Determine session type based on messenger context
    // Direct/DM messages are treated as main session, channels/groups as group session
    let session_type = if msg.is_direct {
        // Direct messages have full access to MEMORY.md etc.
        SessionType::Main
    } else if msg.channel.is_some() {
        // Channel/group messages have restricted access for privacy
        SessionType::Group
    } else {
        // Fallback to Main for messages without channel (shouldn't happen)
        SessionType::Main
    };

    // Build workspace context
    let workspace_ctx =
        WorkspaceContext::with_config(config.workspace_dir(), config.workspace_context.clone());
    eprintln!(
        "DEBUG: Building workspace context for session_type={:?}, workspace_dir={}",
        session_type,
        config.workspace_dir().display()
    );
    let workspace_prompt = workspace_ctx.build_context(session_type);
    eprintln!(
        "DEBUG: Workspace prompt length: {} chars",
        workspace_prompt.len()
    );

    // Combine base prompt, safety, workspace context, and messaging context
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
    if let Some(task_section) =
        crate::task_handler::generate_task_prompt_section(task_mgr, session_key).await
    {
        parts.push(task_section);
    }

    // Add model selection guidance for sub-agents
    let model_guidance = crate::model_handler::generate_model_prompt_section(model_registry).await;
    parts.push(model_guidance);

    // Add comprehensive tool usage guidelines (inspired by OpenClaw's patterns)
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
        .to_string()
    );

    parts.push(format!(
        "## Messaging Context\n\
        - Channel: {}\n\
        - Sender: {}\n\
        - Platform: {}\n\
        \n\
        When responding:\n\
        - Be concise and appropriate for chat\n\
        - You have access to tools — use them when helpful\n\
        - For proactive sends, use the `message` tool\n\
        \n\
        {}",
        msg.channel.as_deref().unwrap_or("direct"),
        msg.sender,
        messenger_type,
        get_platform_formatting_guide(messenger_type)
    ));

    // Add runtime info
    parts.push(format!(
        "## Runtime\n\
        Workspace: {}\n\
        Platform: RustyClaw",
        config.workspace_dir().display()
    ));

    parts.join("\n\n")
}

/// Get platform-specific formatting guidance for the system prompt.
fn get_platform_formatting_guide(messenger_type: &str) -> String {
    match messenger_type {
        "matrix" | "matrix-cli" => "\
### Formatting (Matrix)\n\
- **Markdown supported**: bold, italic, code, links, lists\n\
- Tables render in most clients (Element, FluffyChat)\n\
- Code blocks with syntax highlighting: ```rust\n\
- Headers work but keep them minimal in chat"
            .to_string(),

        "discord" => "\
### Formatting (Discord)\n\
- **Markdown supported**: bold, italic, strikethrough, code, links\n\
- **NO tables** — use bullet lists instead\n\
- Code blocks with syntax highlighting: ```rust\n\
- Wrap multiple URLs in <> to suppress embeds: `<https://example.com>`\n\
- Headers don't render — use **bold** for emphasis"
            .to_string(),

        "telegram" => "\
### Formatting (Telegram)\n\
- **Markdown supported**: bold, italic, code, links\n\
- **NO tables** — use bullet lists instead\n\
- Code blocks work but no syntax highlighting in all clients\n\
- Keep messages concise — long messages get truncated"
            .to_string(),

        "whatsapp" => "\
### Formatting (WhatsApp)\n\
- **Limited formatting**: *bold*, _italic_, ~strikethrough~, ```code```\n\
- **NO markdown links** — just paste URLs directly\n\
- **NO tables, NO headers** — use plain text with line breaks\n\
- **NO bullet points** — use dashes or numbers manually\n\
- Keep it simple and conversational"
            .to_string(),

        "slack" => "\
### Formatting (Slack)\n\
- **Mrkdwn** (not standard markdown): *bold*, _italic_, ~strike~, `code`\n\
- **NO tables** — use bullet lists\n\
- Code blocks: ```code``` (no syntax highlighting)\n\
- Links: <https://url|display text>\n\
- Use emoji reactions when appropriate"
            .to_string(),

        "signal" => "\
### Formatting (Signal)\n\
- **NO formatting support** — plain text only\n\
- Just write naturally without markdown\n\
- URLs will auto-link"
            .to_string(),

        "irc" => "\
### Formatting (IRC)\n\
- **NO formatting** — plain text only\n\
- Keep lines short (typically <400 chars)\n\
- No markdown, no special characters"
            .to_string(),

        _ => "\
### Formatting\n\
- Use plain text to be safe\n\
- Avoid complex markdown unless you know the platform supports it"
            .to_string(),
    }
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
- Use plain language unless in technical context".to_string()
}
