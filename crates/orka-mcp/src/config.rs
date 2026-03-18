use serde::Deserialize;

/// Configuration for an MCP server process.
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    /// Unique name for this MCP server (used in tool qualified names).
    pub name: String,
    /// Executable path or name for the MCP server process.
    pub command: String,
    /// Command-line arguments passed to the server process.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables injected into the server process.
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}
