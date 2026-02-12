//! Agent tool system for RustyClaw.
//!
//! Provides a registry of tools that the language model can invoke, and
//! formatters that serialise the tool definitions into each provider's
//! native schema (OpenAI function-calling, Anthropic tool-use, Google
//! function declarations).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── Tool definitions ────────────────────────────────────────────────────────

/// JSON-Schema-like parameter definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolParam {
    pub name: String,
    pub description: String,
    /// JSON Schema type: "string", "integer", "boolean", "array", "object".
    #[serde(rename = "type")]
    pub param_type: String,
    pub required: bool,
}

/// A tool that the agent can invoke.
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Vec<ToolParam>,
    /// The function that executes the tool, returning a string result or error.
    pub execute: fn(args: &Value) -> Result<String, String>,
}

// ── Tool registry ───────────────────────────────────────────────────────────

/// Return all available tools.
pub fn all_tools() -> Vec<&'static ToolDef> {
    vec![&READ_FILE]
}

// ── Built-in tools ──────────────────────────────────────────────────────────

/// `read_file` — read the contents of a file on disk.
pub static READ_FILE: ToolDef = ToolDef {
    name: "read_file",
    description: "Read the contents of a file. Returns the file text. \
                  Use the optional start_line / end_line parameters to \
                  read a specific range (1-based, inclusive).",
    parameters: vec![],  // filled by init; see `read_file_params()`.
    execute: exec_read_file,
};

/// We need a runtime-constructed param list because `Vec` isn't const.
/// This function is what the registry / formatters actually call.
pub fn read_file_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "path".into(),
            description: "Absolute or relative path to the file to read.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "start_line".into(),
            description: "First line to read (1-based, inclusive). Omit to start from the beginning.".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "end_line".into(),
            description: "Last line to read (1-based, inclusive). Omit to read to the end.".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

fn exec_read_file(args: &Value) -> Result<String, String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: path".to_string())?;

    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read file '{}': {}", path, e))?;

    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    let start = args
        .get("start_line")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).saturating_sub(1)) // 1-based → 0-based
        .unwrap_or(0);

    let end = args
        .get("end_line")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).min(total))
        .unwrap_or(total);

    if start >= total {
        return Err(format!(
            "start_line {} is past end of file ({} lines)",
            start + 1,
            total,
        ));
    }

    let slice = &lines[start..end.min(total)];
    // Prefix each line with its 1-based line number for model context.
    let numbered: Vec<String> = slice
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{:>4} │ {}", start + i + 1, line))
        .collect();

    Ok(numbered.join("\n"))
}

// ── Provider-specific formatters ────────────────────────────────────────────

/// Parameters for a tool, building a JSON Schema `properties` / `required`.
fn params_to_json_schema(params: &[ToolParam]) -> (Value, Value) {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();

    for p in params {
        let mut prop = serde_json::Map::new();
        prop.insert("type".into(), json!(p.param_type));
        prop.insert("description".into(), json!(p.description));
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

// ── Tool execution ──────────────────────────────────────────────────────────

/// Find a tool by name and execute it with the given arguments.
pub fn execute_tool(name: &str, args: &Value) -> Result<String, String> {
    for tool in all_tools() {
        if tool.name == name {
            return (tool.execute)(args);
        }
    }
    Err(format!("Unknown tool: {}", name))
}

// ── Wire types for WebSocket protocol ───────────────────────────────────────

/// A tool call requested by the model (sent gateway → client for display).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// The result of executing a tool (sent gateway → client for display,
/// and also injected back into the conversation for the model).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub id: String,
    pub name: String,
    pub result: String,
    pub is_error: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_file_this_file() {
        let args = json!({ "path": file!(), "start_line": 1, "end_line": 5 });
        let result = exec_read_file(&args);
        assert!(result.is_ok());
        let text = result.unwrap();
        assert!(text.contains("Agent tool system"));
    }

    #[test]
    fn test_read_file_missing() {
        let args = json!({ "path": "/nonexistent/file.txt" });
        let result = exec_read_file(&args);
        assert!(result.is_err());
    }

    #[test]
    fn test_read_file_no_path() {
        let args = json!({});
        let result = exec_read_file(&args);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_execute_tool_dispatch() {
        let args = json!({ "path": file!() });
        let result = execute_tool("read_file", &args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_execute_tool_unknown() {
        let result = execute_tool("no_such_tool", &json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn test_openai_format() {
        let tools = tools_openai();
        assert!(!tools.is_empty());
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "read_file");
        assert!(tools[0]["function"]["parameters"]["properties"]["path"].is_object());
    }

    #[test]
    fn test_anthropic_format() {
        let tools = tools_anthropic();
        assert!(!tools.is_empty());
        assert_eq!(tools[0]["name"], "read_file");
        assert!(tools[0]["input_schema"]["properties"]["path"].is_object());
    }

    #[test]
    fn test_google_format() {
        let tools = tools_google();
        assert!(!tools.is_empty());
        assert_eq!(tools[0]["name"], "read_file");
    }
}
