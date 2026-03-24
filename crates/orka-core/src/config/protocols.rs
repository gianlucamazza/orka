//! Protocol configurations (MCP, A2A, Guardrails).

use crate::config::defaults;
use serde::Deserialize;
use std::collections::HashMap;

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
    #[serde(default = "defaults::default_mcp_transport")]
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

/// MCP OAuth configuration.
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

/// MCP client configuration.
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

/// Agent-to-Agent (A2A) protocol configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct A2aConfig {
    /// Enable A2A discovery.
    #[serde(default = "defaults::default_a2a_discovery_enabled")]
    pub discovery_enabled: bool,
    /// Discovery interval in seconds.
    #[serde(default = "default_discovery_interval_secs")]
    pub discovery_interval_secs: u64,
    /// Known agent endpoints.
    #[serde(default)]
    pub known_agents: Vec<String>,
}

impl Default for A2aConfig {
    fn default() -> Self {
        Self {
            discovery_enabled: defaults::default_a2a_discovery_enabled(),
            discovery_interval_secs: default_discovery_interval_secs(),
            known_agents: Vec::new(),
        }
    }
}

const fn default_discovery_interval_secs() -> u64 {
    300
}

/// Content guardrails configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct GuardrailsConfig {
    /// Enable guardrails.
    #[serde(default = "defaults::default_guardrails_enabled")]
    pub enabled: bool,
    /// Input validation rules.
    #[serde(default)]
    pub input: GuardrailRules,
    /// Output validation rules.
    #[serde(default)]
    pub output: GuardrailRules,
}

impl Default for GuardrailsConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::default_guardrails_enabled(),
            input: GuardrailRules::default(),
            output: GuardrailRules::default(),
        }
    }
}

impl GuardrailsConfig {
    /// Set enabled flag (builder pattern).
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Set input rules (builder pattern).
    pub fn with_input(mut self, rules: GuardrailRules) -> Self {
        self.input = rules;
        self
    }

    /// Set output rules (builder pattern).
    pub fn with_output(mut self, rules: GuardrailRules) -> Self {
        self.output = rules;
        self
    }
}

/// Guardrail validation rules.
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct GuardrailRules {
    /// Blocked keywords.
    #[serde(default)]
    pub blocked_keywords: Vec<String>,
    /// Blocked patterns (regex).
    #[serde(default)]
    pub blocked_patterns: Vec<String>,
    /// PII redaction patterns.
    #[serde(default)]
    pub redact_patterns: Vec<RedactPattern>,
    /// Maximum content length.
    pub max_length: Option<usize>,
    /// LLM-based content moderation settings.
    #[serde(default)]
    pub llm_moderation: LlmModerationConfig,
}

impl GuardrailRules {
    /// Add a blocked keyword (builder pattern).
    pub fn with_blocked_keyword(mut self, keyword: impl Into<String>) -> Self {
        self.blocked_keywords.push(keyword.into());
        self
    }

    /// Set blocked keywords (builder pattern).
    pub fn with_blocked_keywords(mut self, keywords: Vec<String>) -> Self {
        self.blocked_keywords = keywords;
        self
    }

    /// Add a blocked pattern (builder pattern).
    pub fn with_blocked_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.blocked_patterns.push(pattern.into());
        self
    }

    /// Add a redact pattern (builder pattern).
    pub fn with_redact_pattern(mut self, pattern: RedactPattern) -> Self {
        self.redact_patterns.push(pattern);
        self
    }

    /// Set LLM moderation config (builder pattern).
    pub fn with_llm_moderation(mut self, config: LlmModerationConfig) -> Self {
        self.llm_moderation = config;
        self
    }
}

/// LLM-based content moderation configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct LlmModerationConfig {
    /// Enable LLM-based moderation.
    #[serde(default)]
    pub enabled: bool,
    /// Model to use for moderation (e.g., "gpt-4o-mini", "claude-3-haiku").
    #[serde(default = "default_moderation_model")]
    pub model: String,
    /// Minimum confidence threshold (0.0-1.0).
    #[serde(default = "default_moderation_threshold")]
    pub threshold: f32,
    /// Categories to check.
    #[serde(default)]
    pub categories: Vec<ModerationCategory>,
    /// Custom system prompt for moderation.
    pub system_prompt: Option<String>,
}

impl LlmModerationConfig {
    /// Create a new config with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set enabled flag (builder pattern).
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Set model (builder pattern).
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Set threshold (builder pattern).
    pub fn with_threshold(mut self, threshold: f32) -> Self {
        self.threshold = threshold;
        self
    }

    /// Set categories (builder pattern).
    pub fn with_categories(mut self, categories: Vec<ModerationCategory>) -> Self {
        self.categories = categories;
        self
    }

    /// Set system prompt (builder pattern).
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }
}

/// Moderation categories.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ModerationCategory {
    /// Hate speech, discrimination
    Hate,
    /// Harassment, bullying
    Harassment,
    /// Self-harm, suicide
    SelfHarm,
    /// Violence, physical harm
    Violence,
    /// Sexual content
    Sexual,
    /// Dangerous activities
    Dangerous,
    /// Spam, misleading content
    Spam,
    /// Profanity, obscenity
    Profanity,
}

fn default_moderation_model() -> String {
    "gpt-4o-mini".to_string()
}

fn default_moderation_threshold() -> f32 {
    0.7
}

/// PII redaction pattern.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct RedactPattern {
    /// Pattern name.
    pub name: String,
    /// Regex pattern.
    pub pattern: String,
    /// Replacement string.
    #[serde(default = "default_redact_replacement")]
    pub replacement: String,
}

fn default_redact_replacement() -> String {
    "[REDACTED]".to_string()
}
