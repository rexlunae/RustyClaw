//! Load JSON rules from disk and merge across the three-layer overlay.

use crate::builtin::builtin_rules;
use crate::rule::{JsonRule, LayeredRule, RuleOrigin};
use std::collections::HashMap;
use std::path::Path;
use walkdir::WalkDir;

/// Load rules for all three layers, merge by `id` (later layers replace
/// earlier ones), and sort by descending priority.
pub fn load_layered(
    user_dir: Option<&Path>,
    project_dir: Option<&Path>,
) -> Result<Vec<LayeredRule>, LoadError> {
    let mut by_id: HashMap<String, LayeredRule> = HashMap::new();

    for r in builtin_rules() {
        by_id.insert(r.id.clone(), r.into_layered(RuleOrigin::Builtin, None));
    }

    if let Some(dir) = user_dir {
        merge_dir(dir, RuleOrigin::User, &mut by_id)?;
    }
    if let Some(dir) = project_dir {
        merge_dir(dir, RuleOrigin::Project, &mut by_id)?;
    }

    let mut rules: Vec<LayeredRule> = by_id.into_values().collect();
    rules.sort_by(|a, b| {
        b.rule
            .priority
            .cmp(&a.rule.priority)
            .then_with(|| b.precedence().cmp(&a.precedence()))
    });
    Ok(rules)
}

fn merge_dir(
    dir: &Path,
    origin: RuleOrigin,
    by_id: &mut HashMap<String, LayeredRule>,
) -> Result<(), LoadError> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in WalkDir::new(dir).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let bytes = std::fs::read_to_string(path)
            .map_err(|e| LoadError::Io(path.display().to_string(), e))?;
        let rule: JsonRule = serde_json::from_str(&bytes)
            .map_err(|e| LoadError::Parse(path.display().to_string(), e))?;
        by_id.insert(
            rule.id.clone(),
            rule.into_layered(origin, Some(path.display().to_string())),
        );
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("read {0}: {1}")]
    Io(String, #[source] std::io::Error),
    #[error("parse {0}: {1}")]
    Parse(String, #[source] serde_json::Error),
}
