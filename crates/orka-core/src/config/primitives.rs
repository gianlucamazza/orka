//! Primitive configuration types shared across modules.

use serde::Deserialize;

/// Configuration schema version.
pub const CURRENT_CONFIG_VERSION: u32 = 4;

/// A named workspace entry for multi-workspace support.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceEntry {
    /// Unique name for this workspace, used for routing and CLI selection.
    pub name: String,
    /// Filesystem path to the workspace directory.
    pub dir: String,
}

/// Execution mode for agent graphs.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GraphExecutionMode {
    /// Execute agents sequentially in dependency order.
    #[default]
    Sequential,
    /// Execute agents in parallel where possible.
    Parallel,
    /// Agent-driven execution (agents decide when to hand off).
    Autonomous,
}

impl GraphExecutionMode {
    /// Returns true if this mode supports parallel execution.
    #[must_use]
    pub const fn is_parallel(&self) -> bool {
        matches!(self, Self::Parallel)
    }
}

/// Node behaviour in a multi-agent graph.
///
/// Serialized as: `"agent"`, `"router"`, `"fan_out"`, `"fan_in"`.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeKindDef {
    /// Standard agent: runs the LLM tool loop, can hand off to other agents.
    #[default]
    Agent,
    /// Evaluates outgoing edge conditions without calling the LLM.
    Router,
    /// Dispatches to all successors in parallel.
    FanOut,
    /// Waits for predecessors to complete, then synthesizes results via LLM.
    FanIn,
}

/// Strategy for filtering conversation history when an agent receives a
/// handoff.
///
/// For `last_n` use the companion `history_filter_n` field to set the count.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryFilter {
    /// Pass the full conversation history to the receiving agent (default).
    #[default]
    Full,
    /// Pass only the last N messages (set `history_filter_n` to N).
    LastN,
    /// Start with an empty history — the receiving agent gets a fresh context.
    None,
}

/// Log level filter options.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// Trace level - most verbose.
    Trace,
    /// Debug level.
    Debug,
    /// Info level (default).
    #[default]
    Info,
    /// Warning level.
    Warn,
    /// Error level - least verbose.
    Error,
}

impl LogLevel {
    /// Convert to string representation.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Bus backend options.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BusBackend {
    /// Redis Streams backend.
    #[default]
    Redis,
    /// NATS backend (future).
    Nats,
    /// In-memory backend (testing only).
    Memory,
}

/// Sandbox backend options.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SandboxBackend {
    /// Process-based sandbox.
    #[default]
    Process,
    /// WASM-based sandbox.
    Wasm,
}

/// LLM provider types.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LlmProviderType {
    /// Anthropic Claude.
    Anthropic,
    /// `OpenAI` GPT.
    Openai,
    /// Google Gemini.
    Google,
    /// Ollama local models.
    Ollama,
    /// Custom OpenAI-compatible endpoint.
    Custom,
}

impl LlmProviderType {
    /// Returns true if this provider requires an API key.
    #[must_use]
    pub const fn requires_api_key(&self) -> bool {
        matches!(self, Self::Anthropic | Self::Openai | Self::Google)
    }

    /// Returns true if this provider supports streaming.
    #[must_use]
    pub const fn supports_streaming(&self) -> bool {
        // All providers support streaming as of 2026
        true
    }
}

/// Memory backend options.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MemoryBackend {
    /// Auto-detect based on environment.
    #[default]
    Auto,
    /// Redis backend.
    Redis,
    /// In-memory backend (ephemeral).
    Memory,
}

/// Web search provider options.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WebSearchProvider {
    /// No web search.
    #[default]
    None,
    /// Tavily search API.
    Tavily,
    /// Brave search API.
    Brave,
    /// `SearXNG` self-hosted.
    Searxng,
}

impl WebSearchProvider {
    /// Returns true if a search provider is configured.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        !matches!(self, Self::None)
    }
}

/// OS permission levels.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OsPermissionLevel {
    /// Read-only access to filesystem.
    #[default]
    ReadOnly,
    /// Interactive mode (user confirmation required).
    Interact,
    /// Write access allowed.
    Write,
    /// Execute access allowed.
    Execute,
    /// Admin/sudo access allowed.
    Admin,
}

impl OsPermissionLevel {
    /// Convert to string representation.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Interact => "interact",
            Self::Write => "write",
            Self::Execute => "execute",
            Self::Admin => "admin",
        }
    }

    /// Returns true if this level allows write operations.
    #[must_use]
    pub const fn allows_write(&self) -> bool {
        matches!(self, Self::Write | Self::Execute | Self::Admin)
    }

    /// Returns true if this level allows execution.
    #[must_use]
    pub const fn allows_execute(&self) -> bool {
        matches!(self, Self::Execute | Self::Admin)
    }

    /// Returns true if this level allows sudo.
    #[must_use]
    pub const fn allows_sudo(&self) -> bool {
        matches!(self, Self::Admin)
    }
}

/// MCP transport types.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpTransportType {
    /// Standard I/O transport.
    #[default]
    Stdio,
    /// HTTP streamable transport.
    StreamableHttp,
}

/// Embedding model providers.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingProvider {
    /// Local embeddings with fastembed.
    #[default]
    Local,
    /// `OpenAI` embeddings API.
    Openai,
    /// Anthropic embeddings API.
    Anthropic,
    /// Custom embeddings endpoint.
    Custom,
}

/// Vector store backends.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VectorStoreBackend {
    /// Qdrant vector store.
    #[default]
    Qdrant,
    /// In-memory vector store (testing).
    Memory,
}

/// Experience storage backends.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExperienceBackend {
    /// In-memory storage (ephemeral).
    #[default]
    Memory,
    /// Redis storage.
    Redis,
    /// Disk storage.
    Disk,
}

/// Thinking/reasoning effort level for LLM extended reasoning.
///
/// Maps to Anthropic adaptive thinking and `OpenAI` reasoning effort.
/// Omit `thinking` in agent config to disable thinking entirely.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingEffort {
    /// Minimal thinking — fastest, for simple queries.
    Low,
    /// Moderate thinking — balanced default.
    Medium,
    /// Deep thinking — for complex tasks.
    High,
    /// Maximum depth — only available on Claude Opus 4.6+.
    Max,
}

impl ThinkingEffort {
    /// Return the canonical string value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }
}
