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
#[non_exhaustive]
pub enum McpTransportConfig {
    /// Stdio transport: spawn a child process and communicate over stdin/stdout.
    Stdio {
        /// Executable path or name.
        command: String,
        /// Command-line arguments.
        args: Vec<String>,
        /// Environment variables injected into the process.
        env: HashMap<String, String>,
        /// Working directory for the spawned process. Inherits the server CWD when `None`.
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

    /// Create a new StreamableHttp builder.
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
    pub fn envs(mut self, envs: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>) -> Self {
        self.env.extend(envs.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }

    /// Set the working directory.
    pub fn working_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    /// Build the McpTransportConfig.
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

    /// Build the McpTransportConfig.
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
