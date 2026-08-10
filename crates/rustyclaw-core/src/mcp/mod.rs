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
// The config types are plain serde structs with no MCP-runtime dependency;
// they stay ungated so `[mcp.servers.*]` in rustyclaw.toml round-trips even
// in builds without the `mcp` feature.
mod config;
#[cfg(feature = "mcp")]
mod manager;
#[cfg(feature = "mcp")]
mod tools;

#[cfg(feature = "mcp")]
pub use client::McpClient;
pub use config::{McpConfig, McpServerConfig};
#[cfg(feature = "mcp")]
pub use manager::{McpManager, McpServerStatus, McpStatus};
#[cfg(feature = "mcp")]
pub use tools::{McpTool, McpToolCall, McpToolResult};

/// Shared handle to the MCP manager (registered in
/// [`crate::runtime_ctx`] by the gateway at startup).
#[cfg(feature = "mcp")]
pub type SharedMcpManager = std::sync::Arc<tokio::sync::Mutex<McpManager>>;

// Re-export for convenience when feature is disabled
#[cfg(not(feature = "mcp"))]
pub fn mcp_disabled() -> &'static str {
    "MCP support requires the 'mcp' feature. Rebuild with: cargo build --features mcp"
}
