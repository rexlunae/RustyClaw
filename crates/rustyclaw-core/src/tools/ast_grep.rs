// ast-grep tool integration for RustyClaw.
//
// Provides structural code search, lint, and rewriting via ast-grep (sg).
// ast-grep uses tree-sitter AST patterns to match code structure instead of text.

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

        "search" | "run" => {
            let pattern = args
                .get("pattern")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: pattern")?;

            let lang = args.get("lang").and_then(|v| v.as_str());
            let paths = args.get("paths").and_then(|v| v.as_str());
            let rewrite = args.get("rewrite").and_then(|v| v.as_str());
            let context = args.get("context").and_then(|v| v.as_u64()).unwrap_or(0);
            let json_output = args.get("json").and_then(|v| v.as_bool()).unwrap_or(false);

            let mut cmd = format!("ast-grep run -p '{}'", escape_single_quotes(pattern));

            if let Some(l) = lang {
                cmd.push_str(&format!(" -l {}", l));
            }
            if let Some(r) = rewrite {
                cmd.push_str(&format!(" -r '{}'", escape_single_quotes(r)));
            }
            if context > 0 {
                cmd.push_str(&format!(" -C {}", context));
            }
            if json_output {
                cmd.push_str(" --json=compact");
            }
            if let Some(p) = paths {
                cmd.push_str(&format!(" {}", p));
            }

            sh_in(workspace_dir, &cmd)
        }

        "scan" => {
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
            let base_dir = args
                .get("base_dir")
                .and_then(|v| v.as_str())
                .unwrap_or(".");

            let mut cmd = format!("ast-grep new {} {}", item_type, name);
            if let Some(l) = lang {
                cmd.push_str(&format!(" -l {}", l));
            }
            cmd.push_str(&format!(" -b {}", base_dir));
            cmd.push_str(" -y");

            sh_in(workspace_dir, &cmd)
        }

        "version" => {
            sh("ast-grep --version 2>&1")
        }

        "help" => {
            let subcommand = args.get("subcommand").and_then(|v| v.as_str());
            let cmd = if let Some(s) = subcommand {
                format!("ast-grep {} --help 2>&1", s)
            } else {
                "ast-grep --help 2>&1".to_string()
            };
            sh_in(workspace_dir, &cmd)
        }

        _ => Err(format!(
            "Unknown ast-grep action: '{}'. Valid actions: setup, search, run, scan, test, new, version, help.",
            action
        )),
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn do_setup() -> Result<String, String> {
    if is_installed() {
        let version = sh("ast-grep --version 2>&1")
            .unwrap_or_else(|_| "unknown".into());
        return Ok(format!("ast-grep is already installed ({}).", version.trim()));
    }
    // Install via cargo
    let result = sh("cargo install ast-grep --locked 2>&1")?;
    if is_installed() {
        let version = sh("ast-grep --version 2>&1").unwrap_or_default();
        Ok(format!("ast-grep installed successfully. {}\n{}", version.trim(), result))
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

fn escape_single_quotes(s: &str) -> String {
    s.replace('\'', "'\\''")
}

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
