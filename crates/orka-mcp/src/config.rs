use std::collections::HashMap;

/// OAuth 2.1 Client Credentials configuration for an MCP server.
#[derive(Debug, Clone)]
pub struct McpOAuthConfig {
    /// Token endpoint URL.
    pub token_url: String,
    /// OAuth client ID.
    pub client_id: String,
    /// Name of the environment variable holding the client secret.
    pub client_secret_env: String,
    /// Scopes to request.
    pub scopes: Vec<String>,
}

/// Transport variant for an MCP server connection.
#[derive(Debug, Clone)]
pub enum McpTransportConfig {
    /// Stdio transport: spawn a child process and communicate over stdin/stdout.
    Stdio {
        /// Executable path or name.
        command: String,
        /// Command-line arguments.
        args: Vec<String>,
        /// Environment variables injected into the process.
        env: HashMap<String, String>,
    },
    /// Streamable HTTP transport (MCP spec 2025-03-26).
    StreamableHttp {
        /// Base URL of the MCP endpoint.
        url: String,
        /// Optional OAuth 2.1 Client Credentials config.
        auth: Option<McpOAuthConfig>,
    },
}

/// Configuration for a single MCP server connection.
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    /// Unique name for this MCP server (used to prefix tool names).
    pub name: String,
    /// Transport to use for this server.
    pub transport: McpTransportConfig,
}
