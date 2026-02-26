// Agent setup orchestrator for RustyClaw.
//
// Orchestrates the installation and configuration of all local-model
// infrastructure: uv (Python env), exo (distributed cluster), and
// ollama (local model server).  Can be invoked via `/agent setup`,
// the `rustyclaw setup` CLI, or as an agent-callable tool.

use serde_json::{Value, json};
use std::path::Path;

/// `agent_setup` — install + verify uv, exo, and ollama in one shot.
///
/// Optional `components` array lets the caller pick a subset:
///   `["uv"]`, `["ollama","exo"]`, etc.  Default: all three.
pub fn exec_agent_setup(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let setup_args = json!({"action": "setup"});

    // Which components to set up
    let all = ["uv", "exo", "ollama"];
    let components: Vec<&str> = if let Some(arr) = args.get("components").and_then(|v| v.as_array())
    {
        arr.iter().filter_map(|v| v.as_str()).collect()
    } else {
        all.to_vec()
    };

    let mut results: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for component in &components {
        match *component {
            "uv" => match crate::tools::uv::exec_uv_manage(&setup_args, workspace_dir) {
                Ok(msg) => results.push(format!("✓ uv: {}", msg)),
                Err(e) => errors.push(format!("✗ uv: {}", e)),
            },
            "exo" => match crate::tools::exo_ai::exec_exo_manage(&setup_args, workspace_dir) {
                Ok(msg) => results.push(format!("✓ exo: {}", msg)),
                Err(e) => errors.push(format!("✗ exo: {}", e)),
            },
            "ollama" => {
                match crate::tools::ollama::exec_ollama_manage(&setup_args, workspace_dir) {
                    Ok(msg) => results.push(format!("✓ ollama: {}", msg)),
                    Err(e) => errors.push(format!("✗ ollama: {}", e)),
                }
            }
            other => {
                errors.push(format!("✗ unknown component: '{}'", other));
            }
        }
    }

    let mut output = String::new();
    output.push_str("Agent setup results:\n");
    for r in &results {
        output.push_str(&format!("  {}\n", r));
    }
    if !errors.is_empty() {
        output.push_str("\nWarnings/errors:\n");
        for e in &errors {
            output.push_str(&format!("  {}\n", e));
        }
    }

    if errors.len() == components.len() {
        Err(output)
    } else {
        Ok(output)
    }
}
