//! MCP (Model Context Protocol) client support.
//!
//! This module provides connectivity to MCP servers, allowing RustyClaw to:
//! - Connect to external MCP tool servers (stdio, SSE, WebSocket)
//! - Discover and call tools exposed by MCP servers
//! - Manage multiple MCP server connections
//!
//! # Configuration
//!
//! MCP servers are configured in `rustyclaw.toml`:
//!
//! ```toml
//! [mcp.servers.filesystem]
//! command = "npx"
//! args = ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/dir"]
//!
//! [mcp.servers.github]
//! command = "npx"
//! args = ["-y", "@modelcontextprotocol/server-github"]
//! env = { GITHUB_TOKEN = "..." }
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use rustyclaw_core::mcp::McpManager;
//!
//! let mut manager = McpManager::new();
//! manager.connect("filesystem", &config).await?;
//!
//! // List available tools
//! let tools = manager.list_tools().await?;
//!
//! // Call a tool
//! let result = manager.call_tool("filesystem", "read_file", args).await?;
//! ```

#[cfg(feature = "mcp")]
mod client;
#[cfg(feature = "mcp")]
mod config;
#[cfg(feature = "mcp")]
mod manager;
#[cfg(feature = "mcp")]
mod tools;

#[cfg(feature = "mcp")]
pub use client::McpClient;
#[cfg(feature = "mcp")]
pub use config::{McpServerConfig, McpConfig};
#[cfg(feature = "mcp")]
pub use manager::McpManager;
#[cfg(feature = "mcp")]
pub use tools::{McpTool, McpToolCall, McpToolResult};

// Re-export for convenience when feature is disabled
#[cfg(not(feature = "mcp"))]
pub fn mcp_disabled() -> &'static str {
    "MCP support requires the 'mcp' feature. Rebuild with: cargo build --features mcp"
}
