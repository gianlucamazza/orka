//! Configuration module for Orka.
//!
//! This module provides type-safe configuration structures with serde deserialization support.
//! It is divided into several submodules based on domain:
//!
//! - [`server`]: HTTP server bind configuration.
//! - [`infrastructure`]: Redis, message bus, queue, session, and memory stores.
//! - [`adapters`]: External platform adapters (Telegram, Discord, Slack, etc.).
//! - [`llm`]: LLM provider and model configuration.
//! - [`agent`]: Per-agent runtime and graph configuration.
//! - [`security`]: Authentication, secrets, and sandboxing.
//! - [`knowledge`]: RAG and vector database configuration.
//! - [`observability`]: Metrics, tracing, and audit logging.
//! - [`system`]: Worker, logging, and scheduler configuration.

use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use std::path::Path;

pub mod adapters;
pub mod agent;
pub mod defaults;
pub mod experience;
pub mod http;
pub mod infrastructure;
pub mod knowledge;
pub mod llm;
pub mod observability;
pub mod primitives;
pub mod prompts;
pub mod protocols;
pub mod security;
pub mod server;
pub mod system;
pub mod tools;
pub mod web;

// Re-export all configuration types for backward compatibility
pub use self::adapters::*;
pub use self::agent::*;
pub use self::experience::*;
pub use self::http::*;
pub use self::infrastructure::*;
pub use self::knowledge::*;
pub use self::llm::*;
pub use self::observability::*;
pub use self::primitives::*;
pub use self::prompts::*;
pub use self::protocols::*;
pub use self::security::*;
pub use self::server::*;
pub use self::system::*;
pub use self::tools::*;
pub use self::web::*;

use crate::migrate;

/// Top-level Orka configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct OrkaConfig {
    /// Config schema version. `0` = legacy/absent; current version = `3`.
    #[serde(default = "defaults::default_config_version")]
    pub config_version: u32,
    /// HTTP server bind configuration.
    #[serde(default)]
    pub server: ServerConfig,
    /// Message bus configuration.
    #[serde(default)]
    pub bus: BusConfig,
    /// Redis connection configuration.
    #[serde(default)]
    pub redis: RedisConfig,
    /// Structured logging configuration.
    #[serde(default)]
    pub logging: LoggingConfig,
    /// Path to the default workspace directory.
    #[serde(default = "defaults::default_workspace_dir")]
    pub workspace_dir: String,
    /// Additional named workspace entries for multi-workspace deployments.
    #[serde(default)]
    pub workspaces: Vec<WorkspaceEntry>,
    /// Name of the workspace to use when no explicit workspace is requested.
    #[serde(default)]
    pub default_workspace: Option<String>,
    /// Channel adapter configuration (Telegram, Discord, Slack, WhatsApp, custom).
    #[serde(default)]
    pub adapters: AdapterConfig,
    /// Worker pool configuration.
    #[serde(default)]
    pub worker: WorkerConfig,
    /// In-memory (Redis) memory store configuration.
    #[serde(default)]
    pub memory: MemoryConfig,
    /// Secret storage configuration.
    #[serde(default)]
    pub secrets: SecretConfig,
    /// HTTP authentication configuration.
    #[serde(default)]
    pub auth: AuthConfig,
    /// Code sandbox configuration.
    #[serde(default)]
    pub sandbox: SandboxConfig,
    /// WASM plugin configuration.
    #[serde(default)]
    pub plugins: PluginConfig,
    /// Soft skills (SKILL.md-based instruction skills) configuration.
    #[serde(default)]
    pub soft_skills: SoftSkillConfig,
    /// Session store configuration.
    #[serde(default)]
    pub session: SessionConfig,
    /// Priority queue configuration.
    #[serde(default)]
    pub queue: QueueConfig,
    /// LLM provider configuration.
    #[serde(default)]
    pub llm: LlmConfig,
    /// Per-agent runtime configuration.
    #[serde(default)]
    pub agent: AgentConfig,
    /// Tool enable/disable configuration.
    #[serde(default)]
    pub tools: ToolsConfig,
    /// Observability (metrics/tracing) configuration.
    #[serde(default)]
    pub observe: ObserveConfig,
    /// Skill invocation audit log configuration.
    #[serde(default)]
    pub audit: AuditConfig,
    /// API gateway rate limiting and deduplication configuration.
    #[serde(default)]
    pub gateway: GatewayConfig,
    /// MCP (Model Context Protocol) server and client configuration.
    #[serde(default)]
    pub mcp: McpConfig,
    /// Content guardrails configuration.
    #[serde(default)]
    pub guardrails: GuardrailsConfig,
    /// Web search and content reading configuration.
    #[serde(default)]
    pub web: WebConfig,
    /// OS integration configuration.
    #[serde(default)]
    pub os: OsConfig,
    /// Agent-to-Agent (A2A) protocol configuration.
    #[serde(default)]
    pub a2a: A2aConfig,
    /// Knowledge base and RAG configuration.
    #[serde(default)]
    pub knowledge: KnowledgeConfig,
    /// Cron-based task scheduler configuration.
    #[serde(default)]
    pub scheduler: SchedulerConfig,
    /// HTTP client configuration.
    #[serde(default)]
    pub http: HttpClientConfig,
    /// Prompt template configuration.
    #[serde(default)]
    pub prompts: PromptsConfig,
    /// Self-learning experience configuration.
    #[serde(default)]
    pub experience: ExperienceConfig,
    /// Multi-agent definitions (replaces single `[agent]` for multi-agent deployments).
    #[serde(default)]
    pub agents: Vec<AgentDef>,
    /// Graph topology for multi-agent execution.
    #[serde(default)]
    pub graph: Option<GraphDef>,
}

impl Default for OrkaConfig {
    fn default() -> Self {
        Self {
            config_version: defaults::default_config_version(),
            server: ServerConfig::default(),
            bus: BusConfig::default(),
            redis: RedisConfig::default(),
            logging: LoggingConfig::default(),
            workspace_dir: defaults::default_workspace_dir().to_string(),
            workspaces: Vec::new(),
            default_workspace: None,
            adapters: AdapterConfig::default(),
            worker: WorkerConfig::default(),
            memory: MemoryConfig::default(),
            secrets: SecretConfig::default(),
            auth: AuthConfig::default(),
            sandbox: SandboxConfig::default(),
            plugins: PluginConfig::default(),
            soft_skills: SoftSkillConfig::default(),
            session: SessionConfig::default(),
            queue: QueueConfig::default(),
            llm: LlmConfig::default(),
            agent: AgentConfig::default(),
            tools: ToolsConfig::default(),
            observe: ObserveConfig::default(),
            audit: AuditConfig::default(),
            gateway: GatewayConfig::default(),
            mcp: McpConfig::default(),
            guardrails: GuardrailsConfig::default(),
            web: WebConfig::default(),
            os: OsConfig::default(),
            a2a: A2aConfig::default(),
            knowledge: KnowledgeConfig::default(),
            scheduler: SchedulerConfig::default(),
            http: HttpClientConfig::default(),
            prompts: PromptsConfig::default(),
            experience: ExperienceConfig::default(),
            agents: Vec::new(),
            graph: None,
        }
    }
}

impl OrkaConfig {
    /// Resolve the config file path.
    pub fn resolve_path(path: Option<&Path>) -> std::path::PathBuf {
        path.map(|p| p.to_path_buf())
            .or_else(|| {
                std::env::var("ORKA_CONFIG")
                    .ok()
                    .map(std::path::PathBuf::from)
            })
            .unwrap_or_else(|| "orka.toml".into())
    }

    /// Load configuration from file + environment variables.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let config_path = Self::resolve_path(path);
        let mut builder = Config::builder();

        if config_path.exists() {
            let raw = std::fs::read_to_string(&config_path)
                .map_err(|e| ConfigError::Foreign(Box::new(e)))?;

            let (migrated, result) =
                migrate::migrate_if_needed(&raw).map_err(|e| ConfigError::Foreign(Box::new(e)))?;

            if let Some(ref res) = result {
                for w in &res.warnings {
                    tracing::warn!(
                        from = res.from_version,
                        to = res.to_version,
                        "config migration: {w}"
                    );
                }
            }
            builder = builder.add_source(File::from_str(&migrated, config::FileFormat::Toml));
        }

        builder = builder.add_source(
            Environment::with_prefix("ORKA")
                .separator("__")
                .try_parsing(true),
        );

        builder
            .build()
            .and_then(|c| c.try_deserialize())
            .map_err(|e| {
                ConfigError::Message(format!(
                    "failed to load orka.toml: {e}\n\
                     Hint: check that the file exists, is valid TOML, and that all required \
                     fields are present. Run `orka config validate` for details."
                ))
            })
    }

    /// Validate the loaded configuration.
    pub fn validate(&mut self) -> crate::Result<()> {
        self.llm.apply_defaults();
        
        self.server.validate()?;
        self.redis.validate()?;
        self.worker.validate()?;
        self.gateway.validate()?;
        self.llm.validate()?;
        self.knowledge.validate()?;
        self.http.validate()?;
        self.experience.validate()?;

        if !Path::new(&self.workspace_dir).is_dir() {
            return Err(crate::Error::Config(format!(
                "workspace_dir '{}' does not exist or is not a directory",
                self.workspace_dir
            )));
        }

        // --- Empty strings ---
        if self.agent.id.is_empty() {
            return Err(crate::Error::Config("agent.id must not be empty".into()));
        }

        // --- Enum-like validation (redundant if using enums in sub-configs, but safe) ---
        if !matches!(
            self.logging.level.to_ascii_lowercase().as_str(),
            "trace" | "debug" | "info" | "warn" | "error"
        ) {
            return Err(crate::Error::Config(format!(
                "logging.level must be one of trace/debug/info/warn/error, got: '{}'",
                self.logging.level
            )));
        }

        if self.os.enabled
            && !matches!(
                self.os.permission_level.as_str(),
                "read-only" | "readonly" | "interact" | "write" | "execute" | "admin"
            )
        {
            return Err(crate::Error::Config(format!(
                "os.permission_level must be one of read-only/interact/write/execute/admin, got: '{:?}'",
                self.os.permission_level
            )));
        }

        // Validate multi-workspace entries
        if !self.workspaces.is_empty() {
            let mut seen_names = std::collections::HashSet::new();
            for ws in &self.workspaces {
                if !seen_names.insert(&ws.name) {
                    return Err(crate::Error::Config(format!(
                        "duplicate workspace name: '{}'",
                        ws.name
                    )));
                }
                if !Path::new(&ws.dir).is_dir() {
                    return Err(crate::Error::Config(format!(
                        "workspace '{}' dir '{}' does not exist or is not a directory",
                        ws.name, ws.dir
                    )));
                }
            }
            if let Some(ref default) = self.default_workspace {
                if !self.workspaces.iter().any(|w| &w.name == default) {
                    return Err(crate::Error::Config(format!(
                        "default_workspace '{}' not found in [[workspaces]]",
                        default
                    )));
                }
            }
        }

        // --- Deprecation warnings ---
        if self.web.api_key.is_some() {
            tracing::warn!(
                "web.api_key is deprecated; use web.api_key_env to avoid leaking credentials in the config file"
            );
        }
        for p in &self.llm.providers {
            if p.api_key.is_some() && p.api_key_env.is_some() {
                tracing::warn!(
                    provider = %p.name,
                    "llm.providers[{}].api_key is set alongside api_key_env; api_key_env takes precedence — consider removing the inline key",
                    p.name
                );
            } else if p.api_key.is_some() {
                tracing::warn!(
                    provider = %p.name,
                    "llm.providers[{}].api_key is deprecated; use api_key_env or api_key_secret to avoid leaking credentials in the config file",
                    p.name
                );
            }
        }

        Ok(())
    }
}
