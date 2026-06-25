//! Provider-specific tool-schema formatters.
//!
//! Converts the internal [`ToolDef`] registry into the JSON tool/function
//! schemas expected by each provider's API (OpenAI, Anthropic, Google).

use serde_json::{Value, json};

use super::params::*;
use super::{ToolDef, ToolParam, all_tools, kernel_tools, mcp_tools, model_tools, task_tools};

// ── Provider-specific formatters ────────────────────────────────────────────

/// Parameters for a tool, building a JSON Schema `properties` / `required`.
fn params_to_json_schema(params: &[ToolParam]) -> (Value, Value) {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();

    for p in params {
        let mut prop = serde_json::Map::new();
        prop.insert("type".into(), json!(p.param_type));
        prop.insert("description".into(), json!(p.description));

        // Arrays need an items schema
        if p.param_type == "array" {
            prop.insert("items".into(), json!({"type": "string"}));
        }

        properties.insert(p.name.clone(), Value::Object(prop));
        if p.required {
            required.push(json!(p.name));
        }
    }

    (Value::Object(properties), Value::Array(required))
}

/// Resolve the parameter list for a tool (static defs use empty vecs
/// because Vec isn't const; we resolve at call time).
fn resolve_params(tool: &ToolDef) -> Vec<ToolParam> {
    if !tool.parameters.is_empty() {
        return tool.parameters.clone();
    }
    match tool.name {
        "read_file" => read_file_params(),
        "write_file" => write_file_params(),
        "edit_file" => edit_file_params(),
        "list_directory" => list_directory_params(),
        "search_files" => search_files_params(),
        "find_files" => find_files_params(),
        "execute_command" => execute_command_params(),
        "web_fetch" => web_fetch_params(),
        "web_search" => web_search_params(),
        "process" => process_params(),
        "memory_search" => memory_search_params(),
        "memory_get" => memory_get_params(),
        "save_memory" => save_memory_params(),
        "search_history" => search_history_params(),
        "add_memory" => add_memory_params(),
        "cron" => cron_params(),
        "sessions_list" => sessions_list_params(),
        "sessions_spawn" => sessions_spawn_params(),
        "sessions_send" => sessions_send_params(),
        "sessions_history" => sessions_history_params(),
        "session_status" => session_status_params(),
        "agents_list" => agents_list_params(),
        "apply_patch" => apply_patch_params(),
        "secrets_list" => secrets_list_params(),
        "secrets_get" => secrets_get_params(),
        "secrets_store" => secrets_store_params(),
        "secrets_set_policy" => secrets_set_policy_params(),
        "gateway" => gateway_params(),
        "message" => message_params(),
        "tts" => tts_params(),
        "image" => image_params(),
        "nodes" => nodes_params(),
        "browser" => browser_params(),
        "canvas" => canvas_params(),
        "skill_list" => skill_list_params(),
        "skill_search" => skill_search_params(),
        "skill_install" => skill_install_params(),
        "skill_info" => skill_info_params(),
        "skill_enable" => skill_enable_params(),
        "skill_link_secret" => skill_link_secret_params(),
        "skill_create" => skill_create_params(),
        "mcp_list" => mcp_tools::mcp_list_params(),
        "mcp_connect" => mcp_tools::mcp_connect_params(),
        "mcp_disconnect" => mcp_tools::mcp_disconnect_params(),
        "task_list" => task_tools::task_list_params(),
        "task_status" => task_tools::task_id_param(),
        "task_foreground" => task_tools::task_id_param(),
        "task_background" => task_tools::task_id_param(),
        "task_cancel" => task_tools::task_id_param(),
        "task_pause" => task_tools::task_id_param(),
        "task_resume" => task_tools::task_id_param(),
        "task_input" => task_tools::task_input_params(),
        "task_describe" => task_tools::task_describe_params(),
        "thread_describe" => thread_describe_params(),
        "set_thread_caption" => set_thread_caption_params(),
        "model_list" => model_tools::model_list_params(),
        "model_enable" => model_tools::model_id_param(),
        "model_disable" => model_tools::model_id_param(),
        "model_set" => model_tools::model_id_param(),
        "model_recommend" => model_tools::model_recommend_params(),
        "disk_usage" => disk_usage_params(),
        "classify_files" => classify_files_params(),
        "system_monitor" => system_monitor_params(),
        "battery_health" => battery_health_params(),
        "app_index" => app_index_params(),
        "cloud_browse" => cloud_browse_params(),
        "browser_cache" => browser_cache_params(),
        "screenshot" => screenshot_params(),
        "clipboard" => clipboard_params(),
        "audit_sensitive" => audit_sensitive_params(),
        "secure_delete" => secure_delete_params(),
        "summarize_file" => summarize_file_params(),
        "ask_user" => ask_user_params(),
        "client_dom_query" => client_dom_query_params(),
        "pkg_manage" => pkg_manage_params(),
        "net_info" => net_info_params(),
        "net_scan" => net_scan_params(),
        "service_manage" => service_manage_params(),
        "user_manage" => user_manage_params(),
        "firewall" => firewall_params(),
        "ollama_manage" => ollama_manage_params(),
        "exo_manage" => exo_manage_params(),
        "uv_manage" => uv_manage_params(),
        "npm_manage" => npm_manage_params(),
        "agent_setup" => agent_setup_params(),
        "pdf" => pdf_params(),
        "swarm_create" => swarm_create_params(),
        "swarm_list" => swarm_list_params(),
        "swarm_status" => swarm_status_params(),
        "swarm_send" => swarm_send_params(),
        "swarm_stop" => swarm_stop_params(),
        "swarm_templates" => swarm_templates_params(),
        "host_info" => kernel_tools::host_info_params(),
        "load_status" => kernel_tools::load_status_params(),
        _ => vec![],
    }
}

/// OpenAI / OpenAI-compatible function-calling format.
///
/// ```json
/// { "type": "function", "function": { "name", "description", "parameters": { … } } }
/// ```
pub fn tools_openai() -> Vec<Value> {
    all_tools()
        .into_iter()
        .map(|t| {
            let params = resolve_params(t);
            let (properties, required) = params_to_json_schema(&params);
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": {
                        "type": "object",
                        "properties": properties,
                        "required": required,
                    }
                }
            })
        })
        .collect()
}

/// Anthropic tool-use format.
///
/// ```json
/// { "name", "description", "input_schema": { … } }
/// ```
pub fn tools_anthropic() -> Vec<Value> {
    all_tools()
        .into_iter()
        .map(|t| {
            let params = resolve_params(t);
            let (properties, required) = params_to_json_schema(&params);
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": {
                    "type": "object",
                    "properties": properties,
                    "required": required,
                }
            })
        })
        .collect()
}

/// Google Gemini function-declaration format.
///
/// ```json
/// { "name", "description", "parameters": { … } }
/// ```
pub fn tools_google() -> Vec<Value> {
    all_tools()
        .into_iter()
        .map(|t| {
            let params = resolve_params(t);
            let (properties, required) = params_to_json_schema(&params);
            json!({
                "name": t.name,
                "description": t.description,
                "parameters": {
                    "type": "object",
                    "properties": properties,
                    "required": required,
                }
            })
        })
        .collect()
}
