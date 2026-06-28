//! Autonomous skill curator: auto-proposes skills after complex tasks and
//! periodically grades/consolidates/prunes the skill library.
//!
//! Two components:
//! 1. A heuristic that, after a task using ≥N tool calls, drafts a skill.
//! 2. A scheduled curator that grades existing skills, merges duplicates,
//!    and prunes low-value ones (modeled on Hermes' ~7-day cycle).

use serde_json::{Value, json};
use std::path::Path;
use tracing::{debug, instrument};

use super::ToolParam;

/// Default threshold: propose a skill after this many tool calls in a single task.
const DEFAULT_TOOL_CALL_THRESHOLD: u32 = 10;

/// Default curation cycle in days.
const DEFAULT_CURATOR_CYCLE_DAYS: u32 = 7;

// ── Skill curator tool executor ─────────────────────────────────────────────

/// Execute the `skill_curator` tool.
#[instrument(skip(args, workspace_dir), fields(action))]
pub fn exec_skill_curator(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    tracing::Span::current().record("action", action);
    debug!("Executing skill_curator tool");

    match action {
        "propose" => exec_propose(args, workspace_dir),
        "curate" => exec_curate(args, workspace_dir),
        "grade" => exec_grade(args, workspace_dir),
        "merge" => exec_merge(args, workspace_dir),
        "prune" => exec_prune(args, workspace_dir),
        "status" => exec_status(args, workspace_dir),
        _ => Err(format!(
            "Unknown action: '{}'. Valid: propose, curate, grade, merge, prune, status",
            action
        )),
    }
}

/// Propose a new skill based on a completed multi-step task.
fn exec_propose(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let task_summary = args
        .get("task_summary")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: task_summary")?;

    let tool_calls_used = args
        .get("tool_calls_used")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let threshold = args
        .get("threshold")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_TOOL_CALL_THRESHOLD as u64) as u32;

    let skill_name = args
        .get("skill_name")
        .and_then(|v| v.as_str());

    let skill_body = args
        .get("skill_body")
        .and_then(|v| v.as_str());

    debug!(
        tool_calls_used,
        threshold,
        "Evaluating skill proposal"
    );

    // If tool calls are below threshold and no explicit skill content, suggest skipping
    if tool_calls_used < threshold && skill_body.is_none() {
        return Ok(json!({
            "action": "propose",
            "proposed": false,
            "reason": format!(
                "Task used {} tool calls (below threshold of {}). \
                 Provide skill_body to override.",
                tool_calls_used, threshold
            ),
        }).to_string());
    }

    // Generate skill name from summary if not provided
    let name = skill_name
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            let slug: String = task_summary
                .chars()
                .take(40)
                .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
                .collect();
            slug.trim_matches('-').to_string()
        });

    // Write a draft skill file to workspace
    let skills_dir = workspace_dir.join("skills");
    let skill_dir = skills_dir.join(&name);
    let skill_file = skill_dir.join("SKILL.md");

    let body = skill_body
        .unwrap_or("<!-- Auto-drafted by skill curator. Review and refine. -->");

    // Create the skill directory structure
    if let Err(e) = std::fs::create_dir_all(&skill_dir) {
        return Err(format!("Failed to create skill directory: {}", e));
    }

    let content = format!(
        "# {}\n\n\
         > Auto-proposed after task: {}\n\
         > Tool calls used: {}\n\n\
         ## Instructions\n\n\
         {}\n",
        name, task_summary, tool_calls_used, body
    );

    if let Err(e) = std::fs::write(&skill_file, &content) {
        return Err(format!("Failed to write skill file: {}", e));
    }

    debug!(name = %name, path = ?skill_file, "Skill proposed");
    Ok(json!({
        "action": "propose",
        "proposed": true,
        "skill_name": name,
        "path": skill_file.to_string_lossy(),
        "tool_calls_used": tool_calls_used,
        "note": "Draft skill created. Use skill_create to formalize, or edit SKILL.md directly.",
    }).to_string())
}

/// Run the full curation cycle: grade → merge duplicates → prune low-value.
fn exec_curate(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let cycle_days = args
        .get("cycle_days")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_CURATOR_CYCLE_DAYS as u64) as u32;

    let dry_run = args
        .get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let skills_dir = workspace_dir.join("skills");

    debug!(cycle_days, dry_run, "Running curation cycle");

    // Scan for skill directories
    let skills = scan_skills(&skills_dir);

    if skills.is_empty() {
        return Ok(json!({
            "action": "curate",
            "skills_found": 0,
            "note": "No skills found to curate.",
        }).to_string());
    }

    // Grade each skill
    let mut grades: Vec<Value> = Vec::new();
    for skill in &skills {
        let grade = grade_skill(skill, &skills_dir);
        grades.push(grade);
    }

    // Identify potential merges (skills with similar names)
    let merge_candidates = find_merge_candidates(&skills);

    // Identify prune candidates (unused/low-quality)
    let prune_candidates: Vec<&str> = grades
        .iter()
        .filter(|g| {
            g.get("score")
                .and_then(|s| s.as_f64())
                .map(|s| s < 0.3)
                .unwrap_or(false)
        })
        .filter_map(|g| g.get("name").and_then(|n| n.as_str()))
        .collect();

    Ok(json!({
        "action": "curate",
        "dry_run": dry_run,
        "cycle_days": cycle_days,
        "skills_found": skills.len(),
        "grades": grades,
        "merge_candidates": merge_candidates,
        "prune_candidates": prune_candidates,
        "note": if dry_run {
            "Dry run — no changes made. Set dry_run=false to apply."
        } else {
            "Curation cycle complete."
        },
    }).to_string())
}

/// Grade a specific skill by name.
fn exec_grade(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: name")?;

    let skills_dir = workspace_dir.join("skills");
    let grade = grade_skill(name, &skills_dir);

    Ok(json!({
        "action": "grade",
        "result": grade,
    }).to_string())
}

/// Merge two skills into one.
fn exec_merge(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let source = args
        .get("source")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: source (skill to merge from)")?;

    let target = args
        .get("target")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: target (skill to merge into)")?;

    let dry_run = args
        .get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let skills_dir = workspace_dir.join("skills");
    let source_file = skills_dir.join(source).join("SKILL.md");
    let target_file = skills_dir.join(target).join("SKILL.md");

    if !source_file.exists() {
        return Err(format!("Source skill not found: {}", source));
    }
    if !target_file.exists() {
        return Err(format!("Target skill not found: {}", target));
    }

    if dry_run {
        Ok(json!({
            "action": "merge",
            "dry_run": true,
            "source": source,
            "target": target,
            "note": "Would merge source into target. Set dry_run=false to apply.",
        }).to_string())
    } else {
        // Read both files and append source content to target
        let source_content = std::fs::read_to_string(&source_file)
            .map_err(|e| format!("Failed to read source: {}", e))?;
        let target_content = std::fs::read_to_string(&target_file)
            .map_err(|e| format!("Failed to read target: {}", e))?;

        let merged = format!(
            "{}\n\n---\n\n## Merged from: {}\n\n{}\n",
            target_content, source, source_content
        );

        std::fs::write(&target_file, &merged)
            .map_err(|e| format!("Failed to write merged skill: {}", e))?;

        // Remove source directory
        let source_dir = skills_dir.join(source);
        let _ = std::fs::remove_dir_all(&source_dir);

        debug!(source, target, "Skills merged");
        Ok(json!({
            "action": "merge",
            "dry_run": false,
            "source": source,
            "target": target,
            "result": "merged",
        }).to_string())
    }
}

/// Prune a skill (remove it from the library).
fn exec_prune(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: name")?;

    let dry_run = args
        .get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let reason = args
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("low quality or unused");

    let skills_dir = workspace_dir.join("skills");
    let skill_dir = skills_dir.join(name);

    if !skill_dir.exists() {
        return Err(format!("Skill not found: {}", name));
    }

    if dry_run {
        Ok(json!({
            "action": "prune",
            "dry_run": true,
            "name": name,
            "reason": reason,
            "note": "Would remove skill. Set dry_run=false to apply.",
        }).to_string())
    } else {
        std::fs::remove_dir_all(&skill_dir)
            .map_err(|e| format!("Failed to remove skill directory: {}", e))?;

        debug!(name, reason, "Skill pruned");
        Ok(json!({
            "action": "prune",
            "dry_run": false,
            "name": name,
            "reason": reason,
            "result": "removed",
        }).to_string())
    }
}

/// Show curator status: last run, next scheduled, skill count.
fn exec_status(_args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let skills_dir = workspace_dir.join("skills");
    let skills = scan_skills(&skills_dir);

    Ok(json!({
        "action": "status",
        "skills_count": skills.len(),
        "skills": skills,
        "curator_cycle_days": DEFAULT_CURATOR_CYCLE_DAYS,
        "tool_call_threshold": DEFAULT_TOOL_CALL_THRESHOLD,
        "note": "Use cron tool to schedule periodic curation: \
                 cron(action='add', job={schedule: {every_ms: 604800000}, \
                 command: 'skill_curator(action=curate)'})",
    }).to_string())
}

// ── Helper functions ────────────────────────────────────────────────────────

/// Scan the skills directory and return skill names.
fn scan_skills(skills_dir: &Path) -> Vec<String> {
    if !skills_dir.exists() {
        return Vec::new();
    }

    let mut skills = Vec::new();
    if let Ok(entries) = std::fs::read_dir(skills_dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Some(name) = entry.file_name().to_str() {
                    // Only include directories that have a SKILL.md
                    if entry.path().join("SKILL.md").exists() {
                        skills.push(name.to_string());
                    }
                }
            }
        }
    }
    skills.sort();
    skills
}

/// Grade a skill based on heuristics (file size, structure, age).
fn grade_skill(name: &str, skills_dir: &Path) -> Value {
    let skill_file = skills_dir.join(name).join("SKILL.md");

    if !skill_file.exists() {
        return json!({
            "name": name,
            "score": 0.0,
            "reason": "SKILL.md not found",
        });
    }

    let content = std::fs::read_to_string(&skill_file).unwrap_or_default();
    let lines = content.lines().count();
    let has_instructions = content.contains("## Instructions")
        || content.contains("## Steps")
        || content.contains("## Procedure");
    let has_verification = content.contains("## Verification")
        || content.contains("## Test")
        || content.contains("## Validate");
    let is_auto_draft = content.contains("Auto-drafted by skill curator");

    // Score: 0.0 to 1.0
    let mut score: f64 = 0.0;

    // Length contributes up to 0.3
    score += (lines as f64 / 50.0).min(0.3);

    // Structure contributes up to 0.4
    if has_instructions {
        score += 0.25;
    }
    if has_verification {
        score += 0.15;
    }

    // Penalty for auto-drafts that haven't been refined
    if is_auto_draft && lines < 15 {
        score *= 0.5;
    } else {
        // Non-draft or refined gets a bonus
        score += 0.3;
    }

    score = score.min(1.0);

    json!({
        "name": name,
        "score": (score * 100.0).round() / 100.0,
        "lines": lines,
        "has_instructions": has_instructions,
        "has_verification": has_verification,
        "is_auto_draft": is_auto_draft,
    })
}

/// Find pairs of skills that might be candidates for merging.
fn find_merge_candidates(skills: &[String]) -> Vec<Vec<&str>> {
    let mut candidates = Vec::new();

    for (i, a) in skills.iter().enumerate() {
        for b in skills.iter().skip(i + 1) {
            // Simple heuristic: check if names share a significant prefix
            let common_prefix_len = a
                .chars()
                .zip(b.chars())
                .take_while(|(x, y)| x == y)
                .count();

            let min_len = a.len().min(b.len());
            if min_len > 3 && common_prefix_len as f64 / min_len as f64 > 0.6 {
                candidates.push(vec![a.as_str(), b.as_str()]);
            }
        }
    }

    candidates
}

// ── Parameter definitions ───────────────────────────────────────────────────

pub fn skill_curator_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action: 'propose' (draft skill from task), 'curate' (run full cycle), \
                          'grade' (score a skill), 'merge' (combine two skills), \
                          'prune' (remove a skill), 'status' (show curator state).".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "task_summary".into(),
            description: "Summary of the completed task (for 'propose').".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "tool_calls_used".into(),
            description: "Number of tool calls used in the task (for 'propose'). \
                          Default threshold: 10.".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "threshold".into(),
            description: "Minimum tool calls to trigger auto-proposal (default: 10).".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "skill_name".into(),
            description: "Name for the proposed skill (auto-generated from summary if omitted).".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "skill_body".into(),
            description: "Markdown body for the skill instructions (for 'propose').".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "name".into(),
            description: "Skill name (for 'grade' and 'prune').".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "source".into(),
            description: "Source skill name to merge from (for 'merge').".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "target".into(),
            description: "Target skill name to merge into (for 'merge').".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "dry_run".into(),
            description: "Preview changes without applying (default: true for curate/merge/prune).".into(),
            param_type: "boolean".into(),
            required: false,
        },
        ToolParam {
            name: "cycle_days".into(),
            description: "Curation cycle period in days (default: 7).".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "reason".into(),
            description: "Reason for pruning a skill.".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}
