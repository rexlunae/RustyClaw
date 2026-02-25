//! MCP tool types for integration with RustyClaw's tool system.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An MCP tool exposed by an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    /// Tool name
    pub name: String,

    /// Human-readable description
    pub description: Option<String>,

    /// JSON Schema for the tool's input parameters
    pub input_schema: Value,

    /// The MCP server that provides this tool
    pub server_name: String,
}

impl McpTool {
    /// Convert to the format expected by LLM providers (OpenAI-style function schema)
    pub fn to_llm_tool_schema(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": format!("mcp_{}_{}", self.server_name, self.name),
                "description": self.description.clone().unwrap_or_default(),
                "parameters": self.input_schema.clone()
            }
        })
    }

    /// Get the prefixed name used in tool calls
    pub fn prefixed_name(&self) -> String {
        format!("mcp_{}_{}", self.server_name, self.name)
    }
}

/// A tool call request to an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolCall {
    /// The MCP server to call
    pub server_name: String,

    /// The tool name on that server
    pub tool_name: String,

    /// Arguments as JSON
    pub arguments: Value,
}

impl McpToolCall {
    /// Parse a prefixed tool name (e.g., "mcp_filesystem_read_file") into server + tool
    pub fn from_prefixed_name(prefixed: &str, arguments: Value) -> Option<Self> {
        let stripped = prefixed.strip_prefix("mcp_")?;
        let (server, tool) = stripped.split_once('_')?;
        Some(Self {
            server_name: server.to_string(),
            tool_name: tool.to_string(),
            arguments,
        })
    }
}

/// Result from an MCP tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResult {
    /// Whether the call succeeded
    pub success: bool,

    /// Result content (array of content items per MCP spec)
    pub content: Vec<McpContent>,

    /// Error message if failed
    pub error: Option<String>,
}

/// MCP content item (text, image, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum McpContent {
    Text {
        text: String,
    },
    Image {
        data: String,
        mime_type: String,
    },
    Resource {
        uri: String,
        mime_type: Option<String>,
        text: Option<String>,
    },
}

impl McpToolResult {
    /// Create a successful text result
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            success: true,
            content: vec![McpContent::Text { text: content.into() }],
            error: None,
        }
    }

    /// Create an error result
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            success: false,
            content: vec![],
            error: Some(message.into()),
        }
    }

    /// Convert to a string suitable for LLM consumption
    pub fn to_llm_string(&self) -> String {
        if let Some(ref err) = self.error {
            return format!("Error: {}", err);
        }

        self.content
            .iter()
            .filter_map(|c| match c {
                McpContent::Text { text } => Some(text.clone()),
                McpContent::Image { mime_type, .. } => Some(format!("[Image: {}]", mime_type)),
                McpContent::Resource { uri, text, .. } => {
                    text.clone().or_else(|| Some(format!("[Resource: {}]", uri)))
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
