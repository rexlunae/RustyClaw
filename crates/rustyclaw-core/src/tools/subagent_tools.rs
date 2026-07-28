//! Subagent profile tools: subagent_list, subagent_create, subagent_delete,
//! and the gateway-routed subagent_run stub.
//!
//! Profile management runs directly in core against the filesystem registry
//! (`<settings_dir>/subagents/`). Actually *running* a subagent needs model
//! credentials and the tool loop, which only the gateway has, so
//! `subagent_run` is intercepted there (see the gateway's
//! `subagent_runner`) and the executor here is a stub — mirroring how
//! `ask_user` is handled.

use crate::subagents::SubagentRegistry;
use crate::tools::error::ToolResult;
use crate::tools::{ToolParam, tool_summary};
use serde_json::Value;
use std::path::Path;
use tracing::{debug, instrument};

/// Build the registry from the gateway-published settings dir.
fn registry() -> Result<SubagentRegistry, String> {
    crate::runtime_ctx::get_settings_dir()
        .map(|dir| SubagentRegistry::new(&dir))
        .ok_or_else(|| {
            "Subagent registry unavailable: no gateway settings dir published".to_string()
        })
}

/// List subagent profiles.
#[instrument(skip(_args, _workspace_dir))]
pub fn exec_subagent_list(_args: &Value, _workspace_dir: &Path) -> ToolResult {
    let reg = registry()?;
    let profiles = reg.list();
    debug!(count = profiles.len(), "Listing subagent profiles");

    let mut output = String::from(
        "Subagent profiles (use with subagent_run; subagent_create adds custom ones):\n\n",
    );
    for p in profiles {
        let kind = if p.builtin { "built-in" } else { "custom" };
        output.push_str(&format!("## {} ({}, {})\n", p.id, p.name, kind));
        if !p.description.is_empty() {
            output.push_str(&format!("{}\n", p.description));
        }
        output.push_str(&format!("Tools: {}\n", p.tools.join(", ")));
        if let Some(model) = &p.model {
            output.push_str(&format!("Model: {}\n", model));
        }
        output.push('\n');
    }
    Ok(output)
}

/// Create a custom subagent profile.
#[instrument(skip(args, _workspace_dir), fields(name))]
pub fn exec_subagent_create(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;
    let system_prompt = args
        .get("system_prompt")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: system_prompt".to_string())?;
    let tools: Vec<String> = args
        .get("tools")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "Missing required parameter: tools".to_string())?
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();

    let id = args.get("id").and_then(|v| v.as_str());
    let description = args
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let model = args.get("model").and_then(|v| v.as_str());
    let max_rounds = args
        .get("max_rounds")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    tracing::Span::current().record("name", name);

    let reg = registry()?;
    let profile = reg
        .create(
            id,
            name,
            description,
            system_prompt,
            &tools,
            model,
            max_rounds,
        )
        .map_err(|e| e.to_string())?;

    Ok(format!(
        "Created subagent profile '{}' ({}) with tools: {}.\n\
         Run it with subagent_run(profile=\"{}\", task=\"...\").",
        profile.id,
        profile.name,
        profile.tools.join(", "),
        profile.id
    ))
}

/// Delete a custom subagent profile.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_subagent_delete(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: id".to_string())?;
    let reg = registry()?;
    reg.delete(id).map_err(|e| e.to_string())?;
    Ok(format!("Deleted subagent profile '{}'.", id))
}

/// Stub executor for `subagent_run` — never called directly. Execution is
/// intercepted by the gateway dispatch loop, which has the model
/// credentials needed to drive the subagent's own tool loop.
pub fn exec_subagent_run_stub(_args: &Value, _workspace_dir: &Path) -> ToolResult {
    Err("subagent_run must be executed via the gateway".into())
}

/// Render the "## Your Toolset" section for a subagent's system prompt.
pub fn describe_toolset(tools: &[String]) -> String {
    let mut out = String::from(
        "## Your Toolset\nYou have exactly these tools — no others exist in this session:\n",
    );
    for tool in tools {
        out.push_str(&format!("- `{}` — {}\n", tool, tool_summary(tool)));
    }
    out
}

// ── Parameter schemas ───────────────────────────────────────────────────────

pub fn subagent_list_params() -> Vec<ToolParam> {
    vec![]
}

pub fn subagent_create_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "name".into(),
            description: "Display name for the profile (e.g. 'License Checker').".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "system_prompt".into(),
            description: "Focused system prompt describing the subagent's single job, \
                          method, and expected report format. This replaces the main \
                          system prompt entirely — keep it tight."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "tools".into(),
            description: "Allowlist of tool names the subagent may use. Keep it minimal — \
                          only what the job needs. Interactive, agent-management, and \
                          installation-level tools are rejected."
                .into(),
            param_type: "array".into(),
            required: true,
        },
        ToolParam {
            name: "id".into(),
            description: "Explicit profile id (lowercase, digits, '-', '_'). \
                          Derived from the name when omitted."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "description".into(),
            description: "One-line description shown in subagent_list.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "model".into(),
            description: "Optional default model for this profile (must belong to the \
                          active provider)."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "max_rounds".into(),
            description: "Optional tool-loop round limit for runs of this profile.".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

pub fn subagent_delete_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "id".into(),
        description: "Id of the custom profile to delete. Built-ins cannot be deleted.".into(),
        param_type: "string".into(),
        required: true,
    }]
}

pub fn subagent_run_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "profile".into(),
            description: "Profile id to run (see subagent_list), e.g. 'code-writer', \
                          'code-reviewer', 'bug-hunter'."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "task".into(),
            description: "The task for the subagent. Be precise and self-contained — the \
                          subagent starts with no conversation history."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "context".into(),
            description: "Context the subagent needs: relevant file paths, prior findings, \
                          constraints, error output. Feed it everything relevant — it \
                          cannot see this conversation."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "label".into(),
            description: "Short label for the session list (e.g. 'review-auth-module').".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "model".into(),
            description: "Model override for this run (must belong to the active provider). \
                          Falls back to the profile's model, then the active model."
                .into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}
