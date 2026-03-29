//! Top-level Orka configuration composition, loading, and validation.
//!
//! `orka-config` owns the composed `OrkaConfig` schema together with file/env
//! loading, validation orchestration, and migration entrypoints. Domain-owned
//! config sections stay in their owning crates and are re-exported here for a
//! single canonical configuration surface.

use std::path::Path;

use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;

mod runtime;

pub use orka_a2a::A2aConfig;
pub use orka_adapter_custom::CustomAdapterConfig;
pub use orka_adapter_discord::DiscordAdapterConfig;
pub use orka_adapter_slack::SlackAdapterConfig;
pub use orka_adapter_telegram::TelegramAdapterConfig;
pub use orka_adapter_whatsapp::WhatsAppAdapterConfig;
pub use orka_auth::{ApiKeyEntry, AuthConfig, JwtAuthConfig};
pub use orka_bus::BusConfig;
pub use orka_chart::ChartConfig;
pub use orka_core::{
    MigrationError, MigrationResult,
    config::{AgentConfig, AgentDef, GraphDef, NodeKindDef, defaults},
    inspect_config_issues, migrate_for_write, migrate_if_needed,
};
pub use orka_experience::ExperienceConfig;
pub use orka_gateway::GatewayConfig;
pub use orka_git::{GitAuthorshipConfig, GitAuthorshipMode, GitConfig, GitWorktreeConfig};
pub use orka_guardrails::{
    GuardrailRules, GuardrailsConfig, LlmModerationConfig, ModerationCategory, RedactPattern,
};
pub use orka_http::HttpClientConfig;
pub use orka_knowledge::{
    ChunkingConfig, EmbeddingProviderKind, EmbeddingsConfig, KnowledgeConfig, RetrievalConfig,
    VectorStoreBackend, VectorStoreConfig,
};
pub use orka_llm::{LlmAuthKind, LlmConfig, LlmProviderConfig};
pub use orka_mcp::{McpAuthEntry, McpClientConfig, McpConfig, McpServerEntry};
pub use orka_memory::MemoryConfig;
pub use orka_observe::{AuditConfig, ObserveConfig};
pub use orka_os::{
    ApprovalPolicy, ClaudeCodeConfig, CodexConfig, CodingConfig, CodingProvider,
    CodingProvidersConfig, CodingSelectionPolicy, OpenCodeConfig, OsConfig, SandboxMode,
    SudoConfig,
};
pub use orka_prompts::PromptsConfig;
pub use orka_research::ResearchConfig;
pub use orka_sandbox::{SandboxConfig, SandboxLimitsConfig};
pub use orka_scheduler::{ScheduledJob, SchedulerConfig};
pub use orka_secrets::{SecretBackend, SecretConfig};
pub use orka_session::SessionConfig;
pub use orka_skills::{PluginCapabilities, PluginConfig, PluginInstanceConfig, SoftSkillConfig};
pub use orka_web::{SearchProviderKind, WebConfig};
pub use runtime::{
    LogLevel, LoggingConfig, QueueConfig, RedisConfig, SYSTEM_CONFIG_PATH, ServerConfig,
    WorkerConfig, WorkspaceEntry,
};

/// Tool enable/disable configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct ToolsConfig {
    /// Globally allowed tools.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Globally denied tools (takes precedence).
    #[serde(default)]
    pub deny: Vec<String>,
    /// Tool-specific configuration.
    #[serde(default)]
    pub config: std::collections::HashMap<String, serde_json::Value>,
}

/// Channel adapter configuration (Telegram, Discord, Slack, `WhatsApp`,
/// custom).
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct AdapterConfig {
    /// Custom HTTP adapter configuration.
    pub custom: Option<CustomAdapterConfig>,
    /// Telegram bot adapter configuration.
    pub telegram: Option<TelegramAdapterConfig>,
    /// Discord bot adapter configuration.
    pub discord: Option<DiscordAdapterConfig>,
    /// Slack bot adapter configuration.
    pub slack: Option<SlackAdapterConfig>,
    /// `WhatsApp` Cloud API adapter configuration.
    pub whatsapp: Option<WhatsAppAdapterConfig>,
}

/// Top-level Orka configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct OrkaConfig {
    /// Config schema version.
    #[serde(default = "runtime::default_config_version")]
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
    /// Channel adapter configuration.
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
    /// Soft skills configuration.
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
    /// Tool enable/disable configuration.
    #[serde(default)]
    pub tools: ToolsConfig,
    /// Observability configuration.
    #[serde(default)]
    pub observe: ObserveConfig,
    /// Skill invocation audit log configuration.
    #[serde(default)]
    pub audit: AuditConfig,
    /// API gateway rate limiting and deduplication configuration.
    #[serde(default)]
    pub gateway: GatewayConfig,
    /// MCP configuration.
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
    /// Agent-to-Agent protocol configuration.
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
    /// Git integration configuration.
    #[serde(default)]
    pub git: GitConfig,
    /// Multi-agent definitions.
    #[serde(default)]
    pub agents: Vec<AgentDef>,
    /// Graph topology for multi-agent execution.
    #[serde(default)]
    pub graph: Option<GraphDef>,
    /// Autonomous research campaign configuration.
    #[serde(default)]
    pub research: ResearchConfig,
    /// Chart generation skill configuration.
    #[serde(default)]
    pub chart: ChartConfig,
}

impl Default for OrkaConfig {
    fn default() -> Self {
        Self {
            config_version: runtime::default_config_version(),

            server: ServerConfig::default(),
            bus: BusConfig::default(),
            redis: RedisConfig::default(),
            logging: LoggingConfig::default(),
            workspace_dir: defaults::default_workspace_dir(),
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
            git: GitConfig::default(),
            agents: Vec::new(),
            graph: None,
            research: ResearchConfig::default(),
            chart: ChartConfig::default(),
        }
    }
}

impl OrkaConfig {
    /// Resolve the config file path.
    pub fn resolve_path(path: Option<&Path>) -> std::path::PathBuf {
        if let Some(p) = path {
            return p.to_path_buf();
        }
        if let Ok(p) = std::env::var("ORKA_CONFIG") {
            return std::path::PathBuf::from(p);
        }
        let cwd = std::path::PathBuf::from("orka.toml");
        if cwd.exists() {
            return cwd;
        }
        let system = std::path::PathBuf::from(SYSTEM_CONFIG_PATH);
        if system.exists() {
            return system;
        }
        cwd
    }

    /// Load configuration from file + environment variables.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let config_path = Self::resolve_path(path);
        let mut builder = Config::builder();

        if config_path.exists() {
            let raw = std::fs::read_to_string(&config_path)
                .map_err(|e| ConfigError::Foreign(Box::new(e)))?;

            let (migrated, result) =
                migrate_if_needed(&raw).map_err(|e| ConfigError::Foreign(Box::new(e)))?;

            if let Some(ref res) = result {
                for warning in &res.warnings {
                    tracing::warn!(
                        from = res.from_version,
                        to = res.to_version,
                        "config migration: {warning}"
                    );
                }
            }
            let schema_issues =
                inspect_config_issues(&migrated).map_err(|e| ConfigError::Foreign(Box::new(e)))?;
            if !schema_issues.is_empty() {
                return Err(ConfigError::Message(format!(
                    "failed to load orka.toml: configuration schema drift detected:\n- {}",
                    schema_issues.join("\n- ")
                )));
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
            .and_then(config::Config::try_deserialize)
            .map_err(|e| {
                ConfigError::Message(format!(
                    "failed to load orka.toml: {e}\n\
                     Hint: check that the file exists, is valid TOML, and that all required \
                     fields are present. Run `orka config validate` for details."
                ))
            })
    }

    /// Validate the loaded configuration.
    pub fn validate(&mut self) -> orka_core::Result<()> {
        self.llm.apply_defaults();
        self.validate_sub_configs()?;
        self.validate_agents()?;
        self.validate_graph()?;
        self.validate_workspaces()?;
        self.warn_deprecations();
        Ok(())
    }

    fn validate_sub_configs(&self) -> orka_core::Result<()> {
        self.server.validate()?;
        self.bus.validate()?;
        self.redis.validate()?;
        self.logging.validate()?;
        self.worker.validate()?;
        self.queue.validate()?;
        self.memory.validate()?;
        self.secrets.validate()?;
        self.gateway.validate()?;
        self.llm.validate()?;
        self.knowledge.validate()?;
        self.http.validate()?;
        self.os.validate()?;
        self.experience.validate()?;
        self.research.validate()?;
        self.scheduler.validate()?;
        self.mcp.validate()?;
        self.auth.validate()?;
        self.sandbox.validate()?;
        self.plugins.validate()?;
        self.soft_skills.validate()?;
        self.session.validate()?;
        self.observe.validate()?;
        self.audit.validate()?;
        self.guardrails.validate()?;
        self.web.validate()?;
        self.a2a.validate()?;
        self.prompts.validate()?;
        self.git.validate()?;
        self.chart.validate()?;
        if let Some(custom) = &self.adapters.custom {
            custom.validate()?;
        }
        if let Some(tg) = &self.adapters.telegram {
            tg.validate()?;
        }
        if let Some(dc) = &self.adapters.discord {
            dc.validate()?;
        }
        if let Some(slack) = &self.adapters.slack {
            slack.validate()?;
        }
        if let Some(wa) = &self.adapters.whatsapp {
            wa.validate()?;
        }
        if !Path::new(&self.workspace_dir).is_dir() {
            return Err(orka_core::Error::Config(format!(
                "workspace_dir '{}' does not exist or is not a directory",
                self.workspace_dir
            )));
        }
        Ok(())
    }

    fn validate_agents(&mut self) -> orka_core::Result<()> {
        if self.agents.is_empty() {
            let mut agent = AgentDef::new(defaults::default_agent_id());
            agent.config = AgentConfig::default();
            self.agents.push(agent);
            self.graph.get_or_insert_with(GraphDef::default);
        } else if self.graph.is_none() {
            return Err(orka_core::Error::Config(
                "[[agents]] is set but [graph] is missing — add [graph] section to config".into(),
            ));
        }
        for agent_def in &self.agents {
            if agent_def.id.is_empty() {
                return Err(orka_core::Error::Config(
                    "agent id must not be empty".into(),
                ));
            }
        }
        Ok(())
    }

    fn validate_graph(&self) -> orka_core::Result<()> {
        let Some(ref graph_def) = self.graph else {
            return Ok(());
        };
        if let Some(ref entry) = graph_def.entry
            && !self.agents.iter().any(|a| &a.id == entry)
        {
            return Err(orka_core::Error::Config(format!(
                "graph.entry '{entry}' does not match any [[agents]] id"
            )));
        }
        let entry_id = graph_def
            .entry
            .as_deref()
            .or_else(|| self.agents.first().map(|a| a.id.as_str()));
        if let Some(eid) = entry_id
            && let Some(entry_def) = self.agents.iter().find(|a| a.id == eid)
            && entry_def.kind == NodeKindDef::FanIn
        {
            return Err(orka_core::Error::Config(format!(
                "entry node '{eid}' cannot be `fan_in` — nothing to aggregate"
            )));
        }
        for agent_def in &self.agents {
            let id = &agent_def.id;
            let outgoing_count = graph_def.edges.iter().filter(|e| &e.from == id).count();
            match agent_def.kind {
                NodeKindDef::Router if outgoing_count == 0 => {
                    return Err(orka_core::Error::Config(format!(
                        "[[agents]] id={id}: `router` node must have at least one outgoing edge"
                    )));
                }
                NodeKindDef::FanOut if outgoing_count < 2 => {
                    return Err(orka_core::Error::Config(format!(
                        "[[agents]] id={id}: `fan_out` node must have at least 2 outgoing edges (has {outgoing_count})"
                    )));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn validate_workspaces(&self) -> orka_core::Result<()> {
        if self.workspaces.is_empty() {
            return Ok(());
        }
        let mut seen_names = std::collections::HashSet::new();
        for ws in &self.workspaces {
            if !seen_names.insert(&ws.name) {
                return Err(orka_core::Error::Config(format!(
                    "duplicate workspace name: '{}'",
                    ws.name
                )));
            }
            if !Path::new(&ws.dir).is_dir() {
                return Err(orka_core::Error::Config(format!(
                    "workspace '{}' dir '{}' does not exist or is not a directory",
                    ws.name, ws.dir
                )));
            }
        }
        if let Some(ref default) = self.default_workspace
            && !self.workspaces.iter().any(|w| &w.name == default)
        {
            return Err(orka_core::Error::Config(format!(
                "default_workspace '{default}' not found in [[workspaces]]"
            )));
        }
        Ok(())
    }

    fn warn_deprecations(&self) {
        if self.web.api_key.is_some() {
            tracing::warn!(
                "web.api_key is deprecated; use web.api_key_env to avoid leaking credentials in the config file"
            );
        }
        for provider in &self.llm.providers {
            if provider.api_key.is_some() && provider.api_key_env.is_some() {
                tracing::warn!(
                    provider = %provider.name,
                    "llm.providers[{}].api_key is set alongside api_key_env; api_key_env takes precedence — consider removing the inline key",
                    provider.name
                );
            } else if provider.api_key.is_some() {
                tracing::warn!(
                    provider = %provider.name,
                    "llm.providers[{}].api_key is deprecated; use api_key_env or api_key_secret to avoid leaking credentials in the config file",
                    provider.name
                );
            }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::default_trait_access,
    clippy::field_reassign_with_default
)]
mod tests {
    use std::fs;

    use super::*;

    fn temp_workspace() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn load_preserves_owner_side_config_sections() {
        let workspace = temp_workspace();
        let config_path = workspace.path().join("orka.toml");
        let config = format!(
            r#"
workspace_dir = "{workspace}"

[git]
enabled = true

[knowledge]
enabled = true
[knowledge.chunking]
chunk_size = 256
chunk_overlap = 32

[web]
search_provider = "brave"

[http]
timeout_secs = 45

[prompts]
templates_dir = "CUSTOM_PROMPTS"
max_principles = 7

[experience]
enabled = true
reflect_on = "all"

[research]
enabled = true
protected_target_branches = ["main", "release/*"]

[chart]
enabled = true

[adapters.telegram]
bot_token_secret = "telegram/bot"
workspace = "ops"

[llm]
default_model = "claude-3-7-sonnet-20250219"

[[llm.providers]]
name = "anthropic"
provider = "anthropic"
auth_kind = "auth_token"
auth_token_env = "ANTHROPIC_AUTH_TOKEN"
"#,
            workspace = workspace.path().display()
        );
        fs::write(&config_path, config).expect("write config");

        let loaded = OrkaConfig::load(Some(&config_path)).expect("load config");

        assert!(loaded.git.enabled);
        assert!(loaded.knowledge.enabled);
        assert_eq!(loaded.knowledge.chunking.chunk_size, 256);
        assert_eq!(loaded.web.search_provider, SearchProviderKind::Brave);
        assert_eq!(loaded.http.timeout_secs, 45);
        assert_eq!(loaded.prompts.templates_dir, "CUSTOM_PROMPTS");
        assert_eq!(loaded.prompts.max_principles, 7);
        assert!(loaded.experience.enabled);
        assert_eq!(loaded.experience.reflect_on, "all");
        assert!(loaded.research.enabled);
        assert_eq!(
            loaded.research.protected_target_branches,
            vec!["main".to_string(), "release/*".to_string()]
        );
        assert!(loaded.chart.enabled);
        assert_eq!(
            loaded
                .adapters
                .telegram
                .as_ref()
                .and_then(|cfg| cfg.workspace.as_deref()),
            Some("ops")
        );
        assert_eq!(loaded.llm.providers.len(), 1);
        assert_eq!(loaded.llm.providers[0].auth_kind, LlmAuthKind::AuthToken);
    }

    #[test]
    fn validate_adds_default_agent_and_graph() {
        let workspace = temp_workspace();
        let mut config = OrkaConfig {
            workspace_dir: workspace.path().display().to_string(),
            ..OrkaConfig::default()
        };

        config.validate().expect("validate config");

        assert_eq!(config.agents.len(), 1);
        assert!(config.graph.is_some());
    }

    #[test]
    fn validate_rejects_duplicate_workspace_names() {
        let workspace = temp_workspace();
        let alpha = workspace.path().join("alpha");
        let beta = workspace.path().join("beta");
        fs::create_dir_all(&alpha).expect("create alpha");
        fs::create_dir_all(&beta).expect("create beta");

        let config = OrkaConfig {
            workspace_dir: workspace.path().display().to_string(),
            workspaces: vec![
                WorkspaceEntry {
                    name: "shared".into(),
                    dir: alpha.display().to_string(),
                },
                WorkspaceEntry {
                    name: "shared".into(),
                    dir: beta.display().to_string(),
                },
            ],
            ..OrkaConfig::default()
        };

        let err = config
            .validate_workspaces()
            .expect_err("duplicate names must fail");
        assert!(err.to_string().contains("duplicate workspace name"));
    }
}
