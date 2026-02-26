use tracing::{debug, instrument, warn};

use super::SharedSkillManager;

/// Dispatch a skill management tool call.
///
/// Like `execute_secrets_tool`, these tools bypass the normal
/// `tools::execute_tool` path because they need access to the shared
/// `SkillManager` that lives in the gateway process.
#[instrument(skip(args, skill_mgr), fields(%name))]
pub async fn execute_skill_tool(
    name: &str,
    args: &serde_json::Value,
    skill_mgr: &SharedSkillManager,
) -> Result<String, String> {
    debug!("Executing skill tool");
    match name {
        "skill_list" => exec_gw_skill_list(args, skill_mgr).await,
        "skill_search" => exec_gw_skill_search(args, skill_mgr).await,
        "skill_install" => exec_gw_skill_install(args, skill_mgr).await,
        "skill_info" => exec_gw_skill_info(args, skill_mgr).await,
        "skill_enable" => exec_gw_skill_enable(args, skill_mgr).await,
        "skill_link_secret" => exec_gw_skill_link_secret(args, skill_mgr).await,
        "skill_create" => exec_gw_skill_create(args, skill_mgr).await,
        _ => {
            warn!("Unknown skill tool requested");
            Err(format!("Unknown skill tool: {}", name))
        }
    }
}

/// List all loaded skills, optionally filtered.
#[instrument(skip(args, skill_mgr))]
pub async fn exec_gw_skill_list(
    args: &serde_json::Value,
    skill_mgr: &SharedSkillManager,
) -> Result<String, String> {
    let filter = args.get("filter").and_then(|v| v.as_str()).unwrap_or("all");

    debug!(filter, "Listing skills");

    let mgr = skill_mgr.lock().await;
    let skills = mgr.get_skills();

    if skills.is_empty() {
        return Ok("No skills loaded.".into());
    }

    let filtered: Vec<_> = skills
        .iter()
        .filter(|s| match filter {
            "enabled" => s.enabled,
            "disabled" => !s.enabled,
            "registry" => matches!(s.source, crate::skills::SkillSource::Registry { .. }),
            _ => true, // "all"
        })
        .collect();

    if filtered.is_empty() {
        return Ok(format!("No skills match filter '{}'.", filter));
    }

    debug!(count = filtered.len(), "Skills matched filter");
    let mut lines = Vec::with_capacity(filtered.len() + 1);
    lines.push(format!("{} skill(s):\n", filtered.len()));
    for s in &filtered {
        let status = if s.enabled { "✓" } else { "✗" };
        let source = match &s.source {
            crate::skills::SkillSource::Local => "local".to_string(),
            crate::skills::SkillSource::Registry { version, .. } => {
                format!("registry v{}", version)
            }
        };
        let secrets = if s.linked_secrets.is_empty() {
            String::new()
        } else {
            format!(" [secrets: {}]", s.linked_secrets.join(", "))
        };

        // Check availability (gate status)
        let gate_result = mgr.check_gates(s);
        let availability = if gate_result.passed {
            "available".to_string()
        } else {
            let mut missing = Vec::new();
            if gate_result.wrong_os {
                missing.push("wrong OS".to_string());
            }
            if !gate_result.missing_bins.is_empty() {
                missing.push(format!(
                    "missing bins: {}",
                    gate_result.missing_bins.join(", ")
                ));
            }
            if !gate_result.missing_env.is_empty() {
                missing.push(format!(
                    "missing env: {}",
                    gate_result.missing_env.join(", ")
                ));
            }
            if missing.is_empty() {
                "unavailable".to_string()
            } else {
                format!("unavailable ({})", missing.join("; "))
            }
        };

        lines.push(format!(
            "  {} {} ({}) — {}{} [{}]\n",
            status,
            s.name,
            source,
            s.description.as_deref().unwrap_or("(no description)"),
            secrets,
            availability,
        ));
    }
    Ok(lines.join(""))
}

/// Search the ClawHub registry.
#[instrument(skip(args, skill_mgr))]
pub async fn exec_gw_skill_search(
    args: &serde_json::Value,
    skill_mgr: &SharedSkillManager,
) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: query".to_string())?;

    debug!(query, "Searching ClawHub registry");

    let mgr = skill_mgr.lock().await;
    let results = mgr.search_registry(query).map_err(|e| {
        warn!(query, error = %e, "Registry search failed");
        e.to_string()
    })?;

    if results.is_empty() {
        debug!(query, "No skills found");
        return Ok(format!("No skills found matching '{}'.", query));
    }

    debug!(query, count = results.len(), "Search results found");
    let mut lines = Vec::with_capacity(results.len() + 1);
    lines.push(format!("{} result(s) for '{}':\n", results.len(), query));
    for r in &results {
        let secrets_note = if r.required_secrets.is_empty() {
            String::new()
        } else {
            format!(" (needs: {})", r.required_secrets.join(", "))
        };
        // Use display_name if available, otherwise name
        let display = if r.display_name.is_empty() {
            &r.name
        } else {
            &r.display_name
        };
        let version_str = if r.version.is_empty() {
            "latest".to_string()
        } else {
            format!("v{}", r.version)
        };
        lines.push(format!(
            "  • {} ({}) {} — {}{}\n",
            display, r.name, version_str, r.description, secrets_note,
        ));
    }
    lines.push("\nTo install: /skill install <skill-name>".to_string());
    Ok(lines.join(""))
}

/// Install a skill from the ClawHub registry.
#[instrument(skip(args, skill_mgr))]
pub async fn exec_gw_skill_install(
    args: &serde_json::Value,
    skill_mgr: &SharedSkillManager,
) -> Result<String, String> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;
    let version = args.get("version").and_then(|v| v.as_str());

    debug!(
        skill = name,
        version = version.unwrap_or("latest"),
        "Installing skill from registry"
    );

    let mut mgr = skill_mgr.lock().await;
    mgr.install_from_registry(name, version).map_err(|e| {
        warn!(skill = name, error = %e, "Failed to install skill");
        e.to_string()
    })?;

    // Reload skills so the new one is available immediately.
    mgr.load_skills().map_err(|e| {
        warn!(error = %e, "Failed to reload skills after install");
        e.to_string()
    })?;

    let version_note = version
        .map(|v| format!(" v{}", v))
        .unwrap_or_else(|| " (latest)".into());
    debug!(skill = name, "Skill installed and loaded");
    Ok(format!(
        "Skill '{}'{} installed from ClawHub and loaded.",
        name, version_note,
    ))
}

/// Show detailed information about a skill.
#[instrument(skip(args, skill_mgr))]
pub async fn exec_gw_skill_info(
    args: &serde_json::Value,
    skill_mgr: &SharedSkillManager,
) -> Result<String, String> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;

    debug!(skill = name, "Getting skill info");

    let mgr = skill_mgr.lock().await;
    mgr.skill_info(name).ok_or_else(|| {
        debug!(skill = name, "Skill not found");
        format!("Skill '{}' not found.", name)
    })
}

/// Enable or disable a skill.
#[instrument(skip(args, skill_mgr))]
pub async fn exec_gw_skill_enable(
    args: &serde_json::Value,
    skill_mgr: &SharedSkillManager,
) -> Result<String, String> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;
    let enabled = args
        .get("enabled")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| "Missing required parameter: enabled".to_string())?;

    debug!(skill = name, enabled, "Setting skill enabled state");

    let mut mgr = skill_mgr.lock().await;
    mgr.set_skill_enabled(name, enabled).map_err(|e| {
        warn!(skill = name, error = %e, "Failed to set skill enabled state");
        e.to_string()
    })?;

    let state = if enabled { "enabled" } else { "disabled" };
    debug!(skill = name, state, "Skill state changed");
    Ok(format!("Skill '{}' is now {}.", name, state))
}

/// Link or unlink a vault credential to a skill.
#[instrument(skip(args, skill_mgr))]
pub async fn exec_gw_skill_link_secret(
    args: &serde_json::Value,
    skill_mgr: &SharedSkillManager,
) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;
    let skill = args
        .get("skill")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: skill".to_string())?;
    let secret = args
        .get("secret")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: secret".to_string())?;

    debug!(action, skill, secret, "Linking/unlinking secret");

    let mut mgr = skill_mgr.lock().await;
    match action {
        "link" => {
            mgr.link_secret(skill, secret).map_err(|e| {
                warn!(skill, secret, error = %e, "Failed to link secret");
                e.to_string()
            })?;
            debug!(skill, secret, "Secret linked");
            Ok(format!("Secret '{}' linked to skill '{}'.", secret, skill,))
        }
        "unlink" => {
            mgr.unlink_secret(skill, secret).map_err(|e| {
                warn!(skill, secret, error = %e, "Failed to unlink secret");
                e.to_string()
            })?;
            debug!(skill, secret, "Secret unlinked");
            Ok(format!(
                "Secret '{}' unlinked from skill '{}'.",
                secret, skill,
            ))
        }
        _ => {
            warn!(action, "Unknown link_secret action");
            Err(format!(
                "Unknown action '{}'. Use 'link' or 'unlink'.",
                action,
            ))
        }
    }
}

/// Create a new skill from name, description, and instructions.
#[instrument(skip(args, skill_mgr))]
pub async fn exec_gw_skill_create(
    args: &serde_json::Value,
    skill_mgr: &SharedSkillManager,
) -> Result<String, String> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;
    let description = args
        .get("description")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: description".to_string())?;
    let instructions = args
        .get("instructions")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: instructions".to_string())?;
    let metadata = args.get("metadata").and_then(|v| v.as_str());

    debug!(skill = name, "Creating new skill");

    let mut mgr = skill_mgr.lock().await;
    let path = mgr
        .create_skill(name, description, instructions, metadata)
        .map_err(|e| {
            warn!(skill = name, error = %e, "Failed to create skill");
            e.to_string()
        })?;

    debug!(skill = name, path = %path.display(), "Skill created");
    Ok(format!(
        "✅ Skill '{}' created at {}\nThe skill is now loaded and available.",
        name,
        path.display(),
    ))
}
