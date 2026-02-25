//! Memory tools: memory_search, memory_get, and save_memory.

use serde_json::Value;
use std::path::Path;
use tracing::{debug, instrument};

/// Default half-life for temporal decay in days.
const DEFAULT_HALF_LIFE_DAYS: f64 = 30.0;

/// Search memory files for relevant content.
///
/// Supports optional recency boosting via temporal decay. Recent memory files
/// are weighted higher using exponential decay with a configurable half-life.
#[instrument(skip(args, workspace_dir), fields(query))]
pub fn exec_memory_search(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: query".to_string())?;

    tracing::Span::current().record("query", query);

    let max_results = args
        .get("maxResults")
        .and_then(|v| v.as_u64())
        .unwrap_or(5) as usize;

    let min_score = args
        .get("minScore")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.1);

    // Recency boost options
    let use_recency = args
        .get("recencyBoost")
        .and_then(|v| v.as_bool())
        .unwrap_or(true); // Enabled by default

    let half_life_days = args
        .get("halfLifeDays")
        .and_then(|v| v.as_f64())
        .unwrap_or(DEFAULT_HALF_LIFE_DAYS);

    debug!(max_results, min_score, use_recency, half_life_days, "Searching memory");

    // Build index and search
    let index = crate::memory::MemoryIndex::index_workspace(workspace_dir)?;
    
    let results = if use_recency {
        index.search_with_decay(query, max_results, half_life_days)
    } else {
        index.search(query, max_results)
    };

    if results.is_empty() {
        return Ok("No matching memories found.".to_string());
    }

    // Filter by minimum score and format results
    let mut output = String::new();
    output.push_str(&format!("Memory search results for: {}\n", query));
    if use_recency {
        output.push_str(&format!("(recency boost enabled, half-life: {} days)\n", half_life_days));
    }
    output.push('\n');

    let mut count = 0;
    for result in results {
        if result.score < min_score {
            continue;
        }
        count += 1;

        // Truncate snippet to ~700 chars
        let snippet = if result.chunk.text.len() > 700 {
            format!("{}...", &result.chunk.text[..700])
        } else {
            result.chunk.text.clone()
        };

        output.push_str(&format!(
            "{}. **{}** (lines {}-{}, score: {:.2})\n",
            count,
            result.chunk.path,
            result.chunk.start_line,
            result.chunk.end_line,
            result.score
        ));
        output.push_str(&format!("{}\n\n", snippet));
        output.push_str(&format!(
            "Source: {}#L{}-L{}\n\n",
            result.chunk.path, result.chunk.start_line, result.chunk.end_line
        ));
    }

    if count == 0 {
        debug!("No results above minimum score threshold");
        return Ok("No matching memories found above the minimum score threshold.".to_string());
    }

    debug!(result_count = count, "Memory search complete");
    Ok(output)
}

/// Read content from a memory file.
#[instrument(skip(args, workspace_dir))]
pub fn exec_memory_get(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: path".to_string())?;

    let from_line = args
        .get("from")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);

    let num_lines = args
        .get("lines")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);

    debug!(path, from_line, num_lines, "Reading memory file");

    crate::memory::read_memory_file(workspace_dir, path, from_line, num_lines)
}

/// Save memory using two-layer consolidation.
///
/// This tool allows the LLM to:
/// 1. Append a timestamped entry to HISTORY.md (searchable log)
/// 2. Optionally update MEMORY.md with curated long-term facts
///
/// The LLM decides what's important enough to persist.
#[instrument(skip(args, workspace_dir))]
pub fn exec_save_memory(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let history_entry = args
        .get("history_entry")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: history_entry".to_string())?;

    let memory_update = args
        .get("memory_update")
        .and_then(|v| v.as_str());

    debug!(
        history_entry_len = history_entry.len(),
        has_memory_update = memory_update.is_some(),
        "Saving memory"
    );

    let config = crate::memory_consolidation::ConsolidationConfig::default();
    let consolidation = crate::memory_consolidation::MemoryConsolidation::new(config);

    // Append to HISTORY.md
    let history_size = consolidation.append_history(workspace_dir, history_entry)?;

    // Update MEMORY.md if provided
    let memory_size = if let Some(content) = memory_update {
        consolidation.update_memory(workspace_dir, content)?
    } else {
        // Read current size
        consolidation
            .read_memory(workspace_dir)
            .map(|s| s.len())
            .unwrap_or(0)
    };

    let mut output = String::new();
    output.push_str("Memory saved successfully.\n\n");
    output.push_str(&format!("- HISTORY.md: {} bytes (entry appended)\n", history_size));
    if memory_update.is_some() {
        output.push_str(&format!("- MEMORY.md: {} bytes (updated)\n", memory_size));
    } else {
        output.push_str("- MEMORY.md: unchanged\n");
    }

    Ok(output)
}

/// Search HISTORY.md for past entries matching a pattern.
#[instrument(skip(args, workspace_dir))]
pub fn exec_search_history(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: pattern".to_string())?;

    let max_results = args
        .get("maxResults")
        .and_then(|v| v.as_u64())
        .unwrap_or(10) as usize;

    debug!(pattern, max_results, "Searching history");

    let config = crate::memory_consolidation::ConsolidationConfig::default();
    let consolidation = crate::memory_consolidation::MemoryConsolidation::new(config);

    let results = consolidation.search_history(workspace_dir, pattern, max_results)?;

    if results.is_empty() {
        return Ok(format!("No entries found in HISTORY.md matching: {}", pattern));
    }

    let mut output = String::new();
    output.push_str(&format!("History entries matching '{}' ({} found):\n\n", pattern, results.len()));

    for entry in results {
        output.push_str(&format!("[{}] {}\n\n", entry.timestamp, entry.text));
    }

    Ok(output)
}
