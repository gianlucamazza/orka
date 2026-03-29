use std::collections::HashMap;

use serde::Deserialize;

/// MCP (Model Context Protocol) server and client configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct McpConfig {
    /// MCP servers to connect to.
    #[serde(default)]
    pub servers: Vec<McpServerEntry>,
    /// MCP client configuration.
    #[serde(default)]
    pub client: McpClientConfig,
}

/// MCP server entry configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct McpServerEntry {
    /// Server name.
    pub name: String,
    /// Transport type.
    #[serde(default = "default_mcp_transport")]
    pub transport: String,
    /// Command to execute (for stdio transport).
    pub command: Option<String>,
    /// Arguments for the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// HTTP URL (for streamable HTTP transport).
    pub url: Option<String>,
    /// Working directory for the command.
    pub working_dir: Option<std::path::PathBuf>,
    /// OAuth configuration.
    pub auth: Option<McpAuthEntry>,
}

/// MCP OAuth configuration for declarative config loading.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct McpAuthEntry {
    /// OAuth token URL.
    pub token_url: String,
    /// OAuth client ID.
    pub client_id: String,
    /// Environment variable containing client secret.
    pub client_secret_env: String,
    /// OAuth scopes.
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// MCP client metadata configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct McpClientConfig {
    /// Client name.
    #[serde(default)]
    pub name: String,
    /// Client version.
    #[serde(default)]
    pub version: String,
}

impl McpConfig {
    /// Validate the MCP configuration.
    pub fn validate(&self) -> orka_core::Result<()> {
        let mut seen_names = std::collections::HashSet::new();
        for server in &self.servers {
            if server.name.is_empty() {
                return Err(orka_core::Error::Config(
                    "mcp server name must not be empty".into(),
                ));
            }
            if !seen_names.insert(&server.name) {
                return Err(orka_core::Error::Config(format!(
                    "mcp server name '{}' is not unique",
                    server.name
                )));
            }
            match server.transport.as_str() {
                "stdio" => {
                    if server.command.as_deref().map_or(true, str::is_empty) {
                        return Err(orka_core::Error::Config(format!(
                            "mcp server '{}': stdio transport requires a non-empty command",
                            server.name
                        )));
                    }
                }
                "streamable_http" | "http" => {
                    if server.url.as_deref().map_or(true, str::is_empty) {
                        return Err(orka_core::Error::Config(format!(
                            "mcp server '{}': http transport requires a non-empty url",
                            server.name
                        )));
                    }
                }
                other => {
                    return Err(orka_core::Error::Config(format!(
                        "mcp server '{}': unknown transport '{other}' (expected 'stdio' or 'streamable_http')",
                        server.name
                    )));
                }
            }
        }
        Ok(())
    }
}

fn default_mcp_transport() -> String {
    "stdio".to_string()
}

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
#[non_exhaustive]
pub enum McpTransportConfig {
    /// Stdio transport: spawn a child process and communicate over
    /// stdin/stdout.
    Stdio {
        /// Executable path or name.
        command: String,
        /// Command-line arguments.
        args: Vec<String>,
        /// Environment variables injected into the process.
        env: HashMap<String, String>,
        /// Working directory for the spawned process. Inherits the server CWD
        /// when `None`.
        working_dir: Option<std::path::PathBuf>,
    },
    /// Streamable HTTP transport (MCP spec 2025-03-26).
    StreamableHttp {
        /// Base URL of the MCP endpoint.
        url: String,
        /// Optional OAuth 2.1 Client Credentials config.
        auth: Option<McpOAuthConfig>,
    },
}

impl McpTransportConfig {
    /// Create a new Stdio builder.
    pub fn stdio(command: impl Into<String>) -> StdioBuilder {
        StdioBuilder::new(command)
    }

    /// Create a new `StreamableHttp` builder.
    pub fn http(url: impl Into<String>) -> HttpBuilder {
        HttpBuilder::new(url)
    }
}

/// Builder for Stdio transport configuration.
#[derive(Debug, Default)]
pub struct StdioBuilder {
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    working_dir: Option<std::path::PathBuf>,
}

impl StdioBuilder {
    fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            ..Default::default()
        }
    }

    /// Add a command-line argument.
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Add multiple command-line arguments.
    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Set an environment variable.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Set multiple environment variables.
    pub fn envs(
        mut self,
        envs: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        self.env
            .extend(envs.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }

    /// Set the working directory.
    pub fn working_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    /// Build the `McpTransportConfig`.
    pub fn build(self) -> McpTransportConfig {
        McpTransportConfig::Stdio {
            command: self.command,
            args: self.args,
            env: self.env,
            working_dir: self.working_dir,
        }
    }
}

/// Builder for Streamable HTTP transport configuration.
#[derive(Debug)]
pub struct HttpBuilder {
    url: String,
    auth: Option<McpOAuthConfig>,
}

impl HttpBuilder {
    fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            auth: None,
        }
    }

    /// Set OAuth configuration.
    pub fn auth(mut self, auth: McpOAuthConfig) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Build the `McpTransportConfig`.
    pub fn build(self) -> McpTransportConfig {
        McpTransportConfig::StreamableHttp {
            url: self.url,
            auth: self.auth,
        }
    }
}

/// Configuration for a single MCP server connection.
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    /// Unique name for this MCP server (used to prefix tool names).
    pub name: String,
    /// Transport to use for this server.
    pub transport: McpTransportConfig,
}
