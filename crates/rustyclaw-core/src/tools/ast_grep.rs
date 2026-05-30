// ast-grep tool integration for RustyClaw.
//
// Provides structural code search, lint, and rewriting via the ast-grep library
// (tree-sitter AST patterns). Uses the native Rust API instead of shelling out
// to the CLI, so this works anywhere the tool compiles.

use ast_grep_core::matcher::Pattern;
use ast_grep_language::{LanguageExt, SupportLang};
use serde_json::Value;
use std::path::Path;
use tracing::instrument;

/// `ast_grep_manage` — structural code search, lint, and rewrite via ast-grep.
#[instrument(skip(args, workspace_dir))]
pub fn exec_ast_grep(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: action")?;

    match action {
        "setup" | "install" => do_setup(),

        "search" => do_search(args, workspace_dir),

        "run" => do_rewrite(args, workspace_dir),

        "scan" => {
            // Still uses CLI since scan requires YAML rule config parsing
            let config = args.get("config").and_then(|v| v.as_str());
            let rule = args.get("rule").and_then(|v| v.as_str());
            let paths = args.get("paths").and_then(|v| v.as_str());
            let json_output = args.get("json").and_then(|v| v.as_bool()).unwrap_or(false);
            let mut cmd = "ast-grep scan".to_string();
            if let Some(c) = config {
                cmd.push_str(&format!(" -c {}", c));
            }
            if let Some(r) = rule {
                cmd.push_str(&format!(" -r {}", r));
            }
            if json_output {
                cmd.push_str(" --json=compact");
            }
            if let Some(p) = paths {
                cmd.push_str(&format!(" {}", p));
            }
            sh_in(workspace_dir, &cmd)
        }

        // Remaining actions still need the CLI
        "test" => {
            let config = args.get("config").and_then(|v| v.as_str());
            let test_dir = args.get("test_dir").and_then(|v| v.as_str());
            let mut cmd = "ast-grep test".to_string();
            if let Some(c) = config {
                cmd.push_str(&format!(" -c {}", c));
            }
            if let Some(t) = test_dir {
                cmd.push_str(&format!(" -t {}", t));
            }
            sh_in(workspace_dir, &cmd)
        }

        "new" => {
            let item_type = args
                .get("item_type")
                .and_then(|v| v.as_str())
                .unwrap_or("rule");
            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: name")?;
            let lang = args.get("lang").and_then(|v| v.as_str());
            let base_dir = args.get("base_dir").and_then(|v| v.as_str()).unwrap_or(".");
            let mut cmd = format!("ast-grep new {} {}", item_type, name);
            if let Some(l) = lang {
                cmd.push_str(&format!(" -l {}", l));
            }
            cmd.push_str(&format!(" -b {}", base_dir));
            cmd.push_str(" -y");
            sh_in(workspace_dir, &cmd)
        }

        "version" => {
            let version = String::from("ast-grep v0.42.3 (ast-grep-core + ast-grep-language)");
            Ok(version)
        }

        "help" => Ok(HELP_TEXT.to_string()),

        _ => Err(format!(
            "Unknown ast-grep action: '{}'. Valid actions: setup, search, run, scan, test, new, version, help.",
            action
        )),
    }
}

// ── Search ──────────────────────────────────────────────────────────────────

fn do_search(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let pattern_str = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: pattern")?;

    let lang_ext: &str = args.get("lang").and_then(|v| v.as_str()).unwrap_or("rs");
    let paths_str = args.get("paths").and_then(|v| v.as_str()).unwrap_or(".");
    let context = args.get("context").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    let lang: SupportLang = parse_lang(lang_ext)?;
    let mut results: Vec<serde_json::Value> = Vec::new();

    // Resolve files using glob
    let files: Vec<std::path::PathBuf> = resolve_files(paths_str, workspace_dir)?;
    if files.is_empty() {
        return Ok("No matching files found.".to_string());
    }

    let pattern = Pattern::new(pattern_str, lang);
    let mut total_matches = 0u64;

    for file_path in &files {
        let source = match std::fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(e) => {
                results.push(serde_json::json!({
                    "file": file_path.display().to_string(),
                    "error": format!("cannot read: {}", e),
                }));
                continue;
            }
        };

        let root = lang.ast_grep(&source);
        let matches: Vec<_> = root.root().find_all(&pattern).collect();

        if matches.is_empty() {
            continue;
        }

        for node_match in &matches {
            let start_pos = node_match.start_pos();
            let end_pos = node_match.end_pos();
            let range = node_match.range();
            let text = node_match.text().to_string();
            let (line, col) = start_pos.byte_point();

            let mut result = serde_json::json!({
                "file": file_path.display().to_string(),
                "line": line + 1,
                "column": col + 1,
                "end_line": end_pos.byte_point().0 + 1,
                "end_column": end_pos.byte_point().1 + 1,
                "text": text,
            });

            if context > 0 {
                let lines: Vec<&str> = source.lines().collect();
                let start_line = start_pos.line().saturating_sub(context);
                let end_line = (end_pos.line() + context).min(lines.len().saturating_sub(1));
                let ctx: Vec<&str> = lines[start_line..=end_line].to_vec();
                result["context"] = serde_json::json!(ctx.join("\n"));
            }

            result["range"] = serde_json::json!([range.start, range.end]);
            total_matches += 1;

            results.push(result);
        }
    }

    let output = serde_json::json!({
        "total": total_matches,
        "files": files.len(),
        "results": results,
    });

    Ok(serde_json::to_string_pretty(&output).unwrap_or_else(|_| output.to_string()))
}

// ── Rewrite ─────────────────────────────────────────────────────────────────

fn do_rewrite(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let pattern_str = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: pattern")?;
    let rewrite_str = args
        .get("rewrite")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: rewrite")?;

    let lang_ext: &str = args.get("lang").and_then(|v| v.as_str()).unwrap_or("rs");
    let paths_str = args.get("paths").and_then(|v| v.as_str()).unwrap_or(".");

    let lang: SupportLang = parse_lang(lang_ext)?;
    let files: Vec<std::path::PathBuf> = resolve_files(paths_str, workspace_dir)?;
    if files.is_empty() {
        return Ok("No matching files found.".to_string());
    }

    let pattern = Pattern::new(pattern_str, lang);
    let mut total_replacements = 0u64;
    let mut modified_files = Vec::new();

    for file_path in &files {
        let source = match std::fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(e) => {
                return Err(format!("Cannot read {}: {}", file_path.display(), e));
            }
        };

        let root = lang.ast_grep(&source);
        let matches: Vec<_> = root.root().find_all(&pattern).collect();

        if matches.is_empty() {
            continue;
        }

        // Use replace_all for each match — collect ranges to avoid borrow issues
        let mut replacements: Vec<(usize, usize, String)> = Vec::new();
        for node_match in &matches {
            let range = node_match.range();
            // Build the replacement text with metavar substitution
            let _env = node_match.get_env();
            // For simple patterns, rewrite_str is used directly.
            // Metavariables like $$MATCH_NAME get substituted by ast-grep internally
            // when using the library's replacer feature. For direct text replacement
            // we just use the range + rewrite_str as-is.
            replacements.push((range.start, range.end, rewrite_str.to_string()));
        }

        total_replacements += replacements.len() as u64;

        // Apply replacements from end to start (preserving offsets)
        let mut new_source = source.clone();
        for (start, end, text) in replacements.iter().rev() {
            new_source.replace_range(*start..*end, text);
        }

        std::fs::write(file_path, &new_source)
            .map_err(|e| format!("Cannot write {}: {}", file_path.display(), e))?;

        modified_files.push(serde_json::json!({
            "file": file_path.display().to_string(),
            "replacements": replacements.len(),
        }));
    }

    let output = serde_json::json!({
        "total_replacements": total_replacements,
        "modified_files": modified_files.len(),
        "files": modified_files,
    });

    Ok(serde_json::to_string_pretty(&output).unwrap_or_else(|_| output.to_string()))
}

// ── Helpers ─────────────────────────────────────────────────────────────────

const HELP_TEXT: &str = r#"ast_grep_manage — Code-Aware Structural Search & Rewrite

This tool uses ast-grep (tree-sitter AST) to find and rewrite code by structure,
not by text. Patterns survive formatting differences.

ACTIONS:
  search   - Find code matching a pattern. Returns JSON with positions and text.
  run      - Find-and-replace code matching a pattern. In-place rewrite.
  scan     - Run YAML lint rules. (uses CLI)
  setup    - Install the ast-grep CLI for scan/test/new actions.
  test     - Run YAML rule tests. (uses CLI)
  new      - Scaffold a new rule or test. (uses CLI)
  version  - Print library version.
  help     - Show this text.

SEARCH PARAMETERS:
  action   "search"                          (required)
  pattern  AST pattern string                (required)
  lang     Language code: rs, py, ts, js, ... (default: rs)
  paths    Glob for target files             (default: ".")
  context  Lines of context per match        (default: 0)

REWRITE PARAMETERS:
  action   "run"                             (required)
  pattern  AST pattern string                (required)
  rewrite  Replacement code string           (required)
  lang     Language code                     (default: rs)
  paths    Glob for target files             (default: ".")

PATTERN SYNTAX:
  $$META   Matches any expression (metavariable)
  $_       Matches anything without capturing
  Write patterns as code, not regex.
  Example: `Some($$ARG)` matches all Some(...) calls.

LANGUAGE CODES:
  rs (Rust), py (Python), ts (TypeScript), js (JavaScript),
  go (Go), java (Java), rb (Ruby), rsx/tsx (React JSX/TSX),
  c (C), cpp (C++), cs (C#), rs (Rust), sh (Bash), yaml, json,
  html, css, php, scala, swift, kt (Kotlin), lua, dart.
"#;

/// Parse a language extension/code into a SupportLang.
fn parse_lang(s: &str) -> Result<SupportLang, String> {
    // Normalize: strip leading dot, lowercase
    let normalized = s.trim_start_matches('.').to_lowercase();

    // SupportLang has a FromStr impl — use it
    normalized
        .parse::<SupportLang>()
        .map_err(|_| {
            format!(
                "Unsupported language: '{}'. Try: rs, py, ts, js, go, java, rb, c, cpp, cs, sh, yaml, json, html, css, php",
                s
            )
        })
}

/// Resolve a glob/file path string into absolute paths.
fn resolve_files(pattern: &str, workspace_dir: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let _cwd = if Path::new(pattern).is_absolute() {
        std::path::PathBuf::from(".")
    } else {
        workspace_dir.to_path_buf()
    };

    let mut files: Vec<std::path::PathBuf> = Vec::new();

    // If it's a direct file path (no glob metacharacters), handle it directly
    let has_glob_chars = pattern.contains('*') || pattern.contains('?') || pattern.contains('[');

    if !has_glob_chars {
        let path = if Path::new(pattern).is_absolute() {
            std::path::PathBuf::from(pattern)
        } else {
            workspace_dir.join(pattern)
        };
        if path.is_dir() {
            // Walk the directory recursively for source files
            if let Ok(entries) = std::fs::read_dir(&path) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_file() {
                        files.push(p);
                    }
                }
            }
        } else if path.is_file() {
            files.push(path);
        }
        // If it's neither, return empty (could be a glob that just happens
        // to have no metachars but doesn't exist — let caller handle it)
        return Ok(files);
    }

    // Use glob crate for actual glob patterns
    let full_pattern = if Path::new(pattern).is_absolute() {
        pattern.to_string()
    } else {
        workspace_dir.join(pattern).display().to_string()
    };

    for entry in glob::glob(&full_pattern).map_err(|e| format!("Bad glob pattern: {}", e))? {
        match entry {
            Ok(p) if p.is_file() => files.push(p),
            Ok(_) => {} // skip directories
            Err(e) => eprintln!("[ast_grep] glob error: {}", e),
        }
    }

    files.sort();
    Ok(files)
}

// ── Setup (CLI install) ─────────────────────────────────────────────────────

fn do_setup() -> Result<String, String> {
    if is_installed() {
        let version = sh("ast-grep --version 2>&1").unwrap_or_else(|_| "unknown".into());
        return Ok(format!(
            "ast-grep CLI is already installed ({}).\n\
             Library API is available for search/run actions without the CLI.\n\
             CLI is only needed for: scan (YAML rules), test, new.",
            version.trim()
        ));
    }
    let result = sh("cargo install ast-grep --locked 2>&1")?;
    if is_installed() {
        let version = sh("ast-grep --version 2>&1").unwrap_or_default();
        Ok(format!(
            "ast-grep CLI installed successfully. {}\n{}",
            version.trim(),
            result
        ))
    } else {
        Err(format!("Installation may have failed.\n{}", result))
    }
}

fn is_installed() -> bool {
    std::process::Command::new("which")
        .arg("ast-grep")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ── Shell helpers (for CLI-only actions) ────────────────────────────────────

fn sh(script: &str) -> Result<String, String> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .output()
        .map_err(|e| format!("shell error: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() && stdout.is_empty() {
        return Err(if stderr.is_empty() {
            format!("Command exited with {}", output.status)
        } else {
            stderr
        });
    }
    if !stderr.is_empty() && !stdout.is_empty() {
        Ok(format!("{}\n[stderr] {}", stdout, stderr))
    } else if !stdout.is_empty() {
        Ok(stdout)
    } else {
        Ok(stderr)
    }
}

fn sh_in(dir: &Path, script: &str) -> Result<String, String> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("shell error: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() && stdout.is_empty() {
        return Err(if stderr.is_empty() {
            format!("Command exited with {}", output.status)
        } else {
            stderr
        });
    }
    if !stderr.is_empty() && !stdout.is_empty() {
        Ok(format!("{}\n[stderr] {}", stdout, stderr))
    } else if !stdout.is_empty() {
        Ok(stdout)
    } else {
        Ok(stderr)
    }
}
