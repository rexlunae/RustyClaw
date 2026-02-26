//! Skill tools: skill_list, skill_search, skill_install, skill_info, skill_enable, skill_link_secret.

use serde_json::Value;
use std::path::Path;
use tracing::{debug, instrument, warn};

/// List all loaded skills with their status.
#[instrument(skip(_args, _workspace_dir))]
pub fn exec_skill_list(_args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    debug!("Listing skills (standalone mode)");
    // Stub — the gateway intercepts this and uses its SkillManager.
    Ok("No skills loaded (standalone mode). Connect to the gateway for full skill support.".into())
}

/// Search the ClawHub registry for installable skills.
#[instrument(skip(args, _workspace_dir), fields(query))]
pub fn exec_skill_search(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: query".to_string())?;

    tracing::Span::current().record("query", query);
    debug!("Searching skills");

    // Point users to the clawhub CLI
    Ok(format!(
        "To search for skills matching '{}':\n\n\
         1. Use the /skill search command: /skill search \"{}\"\n\
         2. Install: /skill install <skill-name>\n\n\
         Or from the CLI: rustyclaw clawhub search \"{}\"\n\
         Browse skills at: https://clawhub.com",
        query, query, query,
    ))
}

/// Install a skill from the ClawHub registry.
#[instrument(skip(args, _workspace_dir), fields(skill))]
pub fn exec_skill_install(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;

    tracing::Span::current().record("skill", name);
    debug!("Install skill requested");

    Ok(format!(
        "To install the '{}' skill:\n\n\
         Use the /skill install command: /skill install {}\n\
         Or from the CLI: rustyclaw clawhub install {}\n\n\
         The skill will be downloaded from ClawHub and installed to your skills directory.",
        name, name, name,
    ))
}

/// Show detailed information about a loaded skill.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_skill_info(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let _name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;

    debug!("Skill info requested (standalone mode)");
    Ok("Skill info requires gateway connection for full details.".into())
}

/// Enable or disable a loaded skill.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_skill_enable(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let _name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;
    let _enabled = args
        .get("enabled")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| "Missing required parameter: enabled".to_string())?;

    warn!("Skill enable/disable requires gateway connection");
    Err("Skill enable/disable requires gateway connection.".into())
}

/// Link or unlink a vault credential to a skill.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_skill_link_secret(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;
    let _skill = args
        .get("skill")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: skill".to_string())?;
    let _secret = args
        .get("secret")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: secret".to_string())?;

    if !matches!(action, "link" | "unlink") {
        warn!(action, "Unknown skill link action");
        return Err(format!(
            "Unknown action '{}'. Use 'link' or 'unlink'.",
            action
        ));
    }

    warn!("Skill secret linking requires gateway connection");
    Err("Skill secret linking requires gateway connection.".into())
}

/// Create a new skill on disk from a name, description, and instructions body.
///
/// In standalone mode this writes directly via SkillManager.  When the
/// gateway is running the request is intercepted there instead, but the
/// same logic applies.
#[instrument(skip(args, workspace_dir), fields(skill))]
pub fn exec_skill_create(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;

    tracing::Span::current().record("skill", name);

    let description = args
        .get("description")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: description".to_string())?;
    let instructions = args
        .get("instructions")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: instructions".to_string())?;
    let metadata = args.get("metadata").and_then(|v| v.as_str());

    debug!("Creating skill");

    // Build the skills directory from the workspace (fallback for standalone)
    let skills_dir = workspace_dir.join("skills");
    let mut mgr = crate::skills::SkillManager::with_dirs(vec![skills_dir]);
    match mgr.create_skill(name, description, instructions, metadata) {
        Ok(path) => {
            debug!(path = %path.display(), "Skill created");
            Ok(format!(
                "✅ Skill '{}' created at {}\n\nThe skill is now loaded and available.",
                name,
                path.display()
            ))
        }
        Err(e) => {
            warn!(error = %e, "Failed to create skill");
            Err(format!("Failed to create skill: {e}"))
        }
    }
}
