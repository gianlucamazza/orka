use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use std::path::Path;

use crate::migrate;

/// A named workspace entry for multi-workspace support.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceEntry {
    pub name: String,
    pub dir: String,
}

/// Top-level Orka configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct OrkaConfig {
    /// Config schema version (0 = legacy/absent, current = 1).
    #[serde(default)]
    pub config_version: u32,
    #[serde(default = "default_server")]
    pub server: ServerConfig,
    #[serde(default)]
    pub bus: BusConfig,
    #[serde(default)]
    pub redis: RedisConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default = "default_workspace_dir")]
    pub workspace_dir: String,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceEntry>,
    #[serde(default)]
    pub default_workspace: Option<String>,
    #[serde(default)]
    pub adapters: AdapterConfig,
    #[serde(default)]
    pub worker: WorkerConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub secrets: SecretConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub plugins: PluginConfig,
    #[serde(default)]
    pub session: SessionConfig,
    #[serde(default)]
    pub queue: QueueConfig,
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub observe: ObserveConfig,
    #[serde(default)]
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub guardrails: GuardrailsConfig,
    #[serde(default)]
    pub web: WebConfig,
    #[serde(default)]
    pub os: OsConfig,
    #[serde(default)]
    pub a2a: A2aConfig,
    #[serde(default)]
    pub knowledge: KnowledgeConfig,
    #[serde(default)]
    pub scheduler: SchedulerConfig,
    #[serde(default)]
    pub http: HttpClientConfig,
    #[serde(default)]
    pub experience: ExperienceConfig,
}

/// Web search and read configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct WebConfig {
    #[serde(default = "default_web_search_provider")]
    pub search_provider: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub searxng_base_url: Option<String>,
    #[serde(default = "default_web_max_results")]
    pub max_results: usize,
    #[serde(default = "default_web_max_read_chars")]
    pub max_read_chars: usize,
    #[serde(default = "default_web_max_content_chars")]
    pub max_content_chars: usize,
    #[serde(default = "default_web_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
    #[serde(default = "default_web_read_timeout_secs")]
    pub read_timeout_secs: u64,
    #[serde(default = "default_web_user_agent")]
    pub user_agent: String,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            search_provider: default_web_search_provider(),
            api_key: None,
            api_key_env: None,
            searxng_base_url: None,
            max_results: default_web_max_results(),
            max_read_chars: default_web_max_read_chars(),
            max_content_chars: default_web_max_content_chars(),
            cache_ttl_secs: default_web_cache_ttl_secs(),
            read_timeout_secs: default_web_read_timeout_secs(),
            user_agent: default_web_user_agent(),
        }
    }
}

fn default_web_search_provider() -> String {
    "none".into()
}

fn default_web_max_results() -> usize {
    5
}

fn default_web_max_read_chars() -> usize {
    20_000
}

fn default_web_max_content_chars() -> usize {
    8_000
}

fn default_web_cache_ttl_secs() -> u64 {
    3600
}

fn default_web_read_timeout_secs() -> u64 {
    15
}

fn default_web_user_agent() -> String {
    "Orka/0.1 (Web Agent)".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BusConfig {
    #[serde(default = "default_bus_backend")]
    pub backend: String,
    /// XREADGROUP BLOCK timeout in milliseconds.
    #[serde(default = "default_bus_block_ms")]
    pub block_ms: u64,
    /// XREADGROUP COUNT per read.
    #[serde(default = "default_bus_batch_size")]
    pub batch_size: usize,
    /// Initial backoff on connection error (seconds).
    #[serde(default = "default_bus_backoff_initial_secs")]
    pub backoff_initial_secs: u64,
    /// Maximum backoff cap (seconds).
    #[serde(default = "default_bus_backoff_max_secs")]
    pub backoff_max_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RedisConfig {
    #[serde(default = "default_redis_url")]
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default)]
    pub json: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AdapterConfig {
    pub custom: Option<CustomAdapterConfig>,
    pub telegram: Option<TelegramAdapterConfig>,
    pub discord: Option<DiscordAdapterConfig>,
    pub slack: Option<SlackAdapterConfig>,
    pub whatsapp: Option<WhatsAppAdapterConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TelegramAdapterConfig {
    pub bot_token_secret: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DiscordAdapterConfig {
    pub bot_token_secret: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackAdapterConfig {
    pub bot_token_secret: Option<String>,
    #[serde(default = "default_slack_port")]
    pub listen_port: u16,
    #[serde(default)]
    pub workspace: Option<String>,
}

impl Default for SlackAdapterConfig {
    fn default() -> Self {
        Self {
            bot_token_secret: None,
            listen_port: default_slack_port(),
            workspace: None,
        }
    }
}

fn default_slack_port() -> u16 {
    3000
}

#[derive(Debug, Clone, Deserialize)]
pub struct WhatsAppAdapterConfig {
    pub access_token_secret: Option<String>,
    pub phone_number_id: Option<String>,
    pub verify_token_secret: Option<String>,
    #[serde(default = "default_whatsapp_port")]
    pub listen_port: u16,
    #[serde(default)]
    pub workspace: Option<String>,
}

impl Default for WhatsAppAdapterConfig {
    fn default() -> Self {
        Self {
            access_token_secret: None,
            phone_number_id: None,
            verify_token_secret: None,
            listen_port: default_whatsapp_port(),
            workspace: None,
        }
    }
}

fn default_whatsapp_port() -> u16 {
    3001
}

#[derive(Debug, Clone, Deserialize)]
pub struct CustomAdapterConfig {
    #[serde(default = "default_custom_host")]
    pub host: String,
    #[serde(default = "default_custom_port")]
    pub port: u16,
    #[serde(default)]
    pub workspace: Option<String>,
}

impl Default for CustomAdapterConfig {
    fn default() -> Self {
        Self {
            host: default_custom_host(),
            port: default_custom_port(),
            workspace: None,
        }
    }
}

fn default_custom_host() -> String {
    "127.0.0.1".into()
}

fn default_custom_port() -> u16 {
    8081
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerConfig {
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default = "default_retry_base_delay_ms")]
    pub retry_base_delay_ms: u64,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            concurrency: default_concurrency(),
            retry_base_delay_ms: default_retry_base_delay_ms(),
        }
    }
}

fn default_retry_base_delay_ms() -> u64 {
    5000
}

fn default_concurrency() -> usize {
    4
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemoryConfig {
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            max_entries: default_max_entries(),
        }
    }
}

fn default_max_entries() -> usize {
    10_000
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SecretConfig {
    /// Environment variable name containing the 32-byte hex-encoded encryption key
    /// for encrypting secrets at rest in Redis. If unset or the env var is missing,
    /// secrets are stored in plaintext (development mode).
    #[serde(default)]
    pub encryption_key_env: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_api_key_header")]
    pub api_key_header: String,
    #[serde(default)]
    pub api_keys: Vec<ApiKeyEntry>,
    #[serde(default)]
    pub jwt: Option<JwtAuthConfig>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key_header: default_api_key_header(),
            api_keys: Vec::new(),
            jwt: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct JwtAuthConfig {
    pub issuer: String,
    #[serde(default)]
    pub audience: Option<String>,
    pub jwks_uri: Option<String>,
    /// Static secret for HS256 (alternative to JWKS).
    pub secret: Option<String>,
}

fn default_api_key_header() -> String {
    "X-Api-Key".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiKeyEntry {
    pub name: String,
    pub key_hash: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SandboxConfig {
    #[serde(default = "default_sandbox_backend")]
    pub backend: String,
    #[serde(default)]
    pub limits: SandboxLimitsConfig,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            backend: default_sandbox_backend(),
            limits: SandboxLimitsConfig::default(),
        }
    }
}

fn default_sandbox_backend() -> String {
    "process".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct SandboxLimitsConfig {
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_memory_bytes")]
    pub max_memory_bytes: usize,
    #[serde(default = "default_max_output_bytes")]
    pub max_output_bytes: usize,
}

impl Default for SandboxLimitsConfig {
    fn default() -> Self {
        Self {
            timeout_secs: default_timeout_secs(),
            max_memory_bytes: default_max_memory_bytes(),
            max_output_bytes: default_max_output_bytes(),
        }
    }
}

fn default_timeout_secs() -> u64 {
    30
}

fn default_max_memory_bytes() -> usize {
    64 * 1024 * 1024 // 64 MB
}

fn default_max_output_bytes() -> usize {
    1024 * 1024 // 1 MB
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PluginConfig {
    pub dir: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionConfig {
    #[serde(default = "default_session_ttl_secs")]
    pub ttl_secs: u64,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            ttl_secs: default_session_ttl_secs(),
        }
    }
}

fn default_session_ttl_secs() -> u64 {
    86400 // 24 hours
}

#[derive(Debug, Clone, Deserialize)]
pub struct QueueConfig {
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
        }
    }
}

fn default_max_retries() -> u32 {
    3
}

#[derive(Debug, Clone, Deserialize)]
pub struct ObserveConfig {
    #[serde(default = "default_observe_backend")]
    pub backend: String,
    #[serde(default = "default_observe_batch_size")]
    pub batch_size: usize,
    #[serde(default = "default_observe_flush_interval_ms")]
    pub flush_interval_ms: u64,
}

impl Default for ObserveConfig {
    fn default() -> Self {
        Self {
            backend: default_observe_backend(),
            batch_size: default_observe_batch_size(),
            flush_interval_ms: default_observe_flush_interval_ms(),
        }
    }
}

fn default_observe_batch_size() -> usize {
    50
}

fn default_observe_flush_interval_ms() -> u64 {
    100
}

fn default_observe_backend() -> String {
    "log".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    #[serde(default = "default_gateway_rate_limit")]
    pub rate_limit: u32,
    #[serde(default = "default_gateway_dedup_ttl_secs")]
    pub dedup_ttl_secs: u64,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            rate_limit: default_gateway_rate_limit(),
            dedup_ttl_secs: default_gateway_dedup_ttl_secs(),
        }
    }
}

fn default_gateway_rate_limit() -> u32 {
    60
}

fn default_gateway_dedup_ttl_secs() -> u64 {
    3600
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    #[serde(default = "default_llm_model")]
    pub model: String,
    #[serde(default = "default_llm_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_llm_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_llm_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_llm_api_version")]
    pub api_version: String,
    #[serde(default)]
    pub providers: Vec<LlmProviderConfig>,
    #[serde(default = "default_llm_context_window_tokens")]
    pub context_window_tokens: u32,
}

impl LlmConfig {
    /// Propagate top-level LLM defaults into providers that don't override them.
    pub fn apply_defaults(&mut self) {
        for p in &mut self.providers {
            if p.timeout_secs.is_none() {
                p.timeout_secs = Some(self.timeout_secs);
            }
            if p.max_tokens.is_none() {
                p.max_tokens = Some(self.max_tokens);
            }
            if p.max_retries.is_none() {
                p.max_retries = Some(self.max_retries);
            }
        }
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            model: default_llm_model(),
            timeout_secs: default_llm_timeout_secs(),
            max_tokens: default_llm_max_tokens(),
            max_retries: default_llm_max_retries(),
            api_version: default_llm_api_version(),
            providers: Vec::new(),
            context_window_tokens: default_llm_context_window_tokens(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmProviderConfig {
    pub name: String,
    pub provider: String, // "anthropic", "openai", "ollama"
    #[serde(default)]
    pub api_key_secret: Option<String>,
    /// Direct API key (not recommended for production — use secrets store instead).
    #[serde(default)]
    pub api_key: Option<String>,
    /// Environment variable name for the API key (e.g. "ANTHROPIC_API_KEY").
    /// Checked before the secret store. If set and the env var exists, skip secret store lookup.
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default = "default_llm_model")]
    pub model: String,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub max_retries: Option<u32>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub prefixes: Vec<String>,
    /// Cost per 1K input tokens in USD (for cost tracking metrics).
    #[serde(default)]
    pub cost_per_1k_input_tokens: Option<f64>,
    /// Cost per 1K output tokens in USD (for cost tracking metrics).
    #[serde(default)]
    pub cost_per_1k_output_tokens: Option<f64>,
}

fn default_llm_model() -> String {
    "claude-sonnet-4-6".into()
}

fn default_llm_timeout_secs() -> u64 {
    30
}

fn default_llm_max_tokens() -> u32 {
    8192
}

fn default_llm_max_retries() -> u32 {
    2
}

fn default_llm_api_version() -> String {
    "2023-06-01".into()
}

fn default_llm_context_window_tokens() -> u32 {
    1_000_000
}

/// Per-agent runtime configuration (migrated from workspace markdown files).
#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_agent_id")]
    pub id: String,
    #[serde(default = "default_agent_display_name")]
    pub display_name: String,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default = "default_agent_max_iterations")]
    pub max_iterations: usize,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub context_window_tokens: Option<u32>,
    #[serde(default)]
    pub max_tokens_per_session: Option<u64>,
    #[serde(default)]
    pub heartbeat_interval_secs: Option<u64>,
    #[serde(default)]
    pub summarization_model: Option<String>,
    #[serde(default)]
    pub summarization_threshold: Option<usize>,
    #[serde(default = "default_agent_max_history_entries")]
    pub max_history_entries: usize,
    /// Maximum characters for a single tool result before truncation (default 50_000).
    #[serde(default = "default_max_tool_result_chars")]
    pub max_tool_result_chars: usize,
    /// Per-skill execution timeout in seconds (default 120).
    #[serde(default = "default_skill_timeout_secs")]
    pub skill_timeout_secs: u64,
    /// Max consecutive failures of the same tool before injecting a self-correction hint (default 2).
    #[serde(default = "default_max_tool_retries")]
    pub max_tool_retries: u32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            id: default_agent_id(),
            display_name: default_agent_display_name(),
            timezone: None,
            max_iterations: default_agent_max_iterations(),
            model: None,
            max_tokens: None,
            context_window_tokens: None,
            max_tokens_per_session: None,
            heartbeat_interval_secs: None,
            summarization_model: None,
            summarization_threshold: None,
            max_history_entries: default_agent_max_history_entries(),
            max_tool_result_chars: default_max_tool_result_chars(),
            skill_timeout_secs: default_skill_timeout_secs(),
            max_tool_retries: default_max_tool_retries(),
        }
    }
}

fn default_agent_id() -> String {
    "orka-default".into()
}

fn default_agent_display_name() -> String {
    "Orka".into()
}

fn default_agent_max_iterations() -> usize {
    10
}

fn default_agent_max_history_entries() -> usize {
    50
}

fn default_max_tool_result_chars() -> usize {
    50_000
}

fn default_skill_timeout_secs() -> u64 {
    120
}

fn default_max_tool_retries() -> u32 {
    2
}

/// Tool enable/disable configuration.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ToolsConfig {
    /// Tools to disable (all enabled by default).
    #[serde(default)]
    pub disabled: Vec<String>,
}

fn default_server() -> ServerConfig {
    ServerConfig {
        host: default_host(),
        port: default_port(),
    }
}

fn default_host() -> String {
    "127.0.0.1".into()
}

fn default_port() -> u16 {
    8080
}

fn default_bus_backend() -> String {
    "redis".into()
}

fn default_redis_url() -> String {
    "redis://127.0.0.1:6379".into()
}

fn default_log_level() -> String {
    "info".into()
}

fn default_workspace_dir() -> String {
    ".".into()
}

impl Default for ServerConfig {
    fn default() -> Self {
        default_server()
    }
}

impl Default for BusConfig {
    fn default() -> Self {
        Self {
            backend: default_bus_backend(),
            block_ms: default_bus_block_ms(),
            batch_size: default_bus_batch_size(),
            backoff_initial_secs: default_bus_backoff_initial_secs(),
            backoff_max_secs: default_bus_backoff_max_secs(),
        }
    }
}

fn default_bus_block_ms() -> u64 {
    5000
}

fn default_bus_batch_size() -> usize {
    10
}

fn default_bus_backoff_initial_secs() -> u64 {
    1
}

fn default_bus_backoff_max_secs() -> u64 {
    30
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: default_redis_url(),
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            json: false,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: Vec<McpServerEntry>,
    #[serde(default)]
    pub serve: Option<McpServeConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServeConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_mcp_serve_transport")]
    pub transport: String,
    #[serde(default)]
    pub sse_port: Option<u16>,
}

fn default_mcp_serve_transport() -> String {
    "stdio".into()
}

impl Default for McpServeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            transport: default_mcp_serve_transport(),
            sse_port: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerEntry {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GuardrailsConfig {
    /// Blocked keywords (case-insensitive). Triggers Block on match.
    #[serde(default)]
    pub blocked_keywords: Vec<String>,
    /// Regex patterns that block content.
    #[serde(default)]
    pub block_patterns: Vec<String>,
    /// Regex patterns that redact content (pattern → replacement).
    #[serde(default)]
    pub redact_patterns: Vec<RedactPattern>,
    /// Enable built-in PII filter (emails, phones, SSNs).
    #[serde(default)]
    pub pii_filter: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RedactPattern {
    pub pattern: String,
    pub replacement: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct A2aConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub url: Option<String>,
}

/// Linux OS integration configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct OsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_os_permission_level")]
    pub permission_level: String,
    #[serde(default = "default_os_allowed_paths")]
    pub allowed_paths: Vec<String>,
    #[serde(default = "default_os_blocked_paths")]
    pub blocked_paths: Vec<String>,
    #[serde(default = "default_os_blocked_commands")]
    pub blocked_commands: Vec<String>,
    #[serde(default)]
    pub allowed_commands: Vec<String>,
    #[serde(default = "default_os_max_file_size_bytes")]
    pub max_file_size_bytes: u64,
    #[serde(default = "default_os_shell_timeout_secs")]
    pub shell_timeout_secs: u64,
    #[serde(default = "default_os_max_output_bytes")]
    pub max_output_bytes: usize,
    #[serde(default = "default_os_max_list_entries")]
    pub max_list_entries: usize,
    #[serde(default = "default_os_sensitive_env_patterns")]
    pub sensitive_env_patterns: Vec<String>,
    #[serde(default)]
    pub sudo: SudoConfig,
}

impl Default for OsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            permission_level: default_os_permission_level(),
            allowed_paths: default_os_allowed_paths(),
            blocked_paths: default_os_blocked_paths(),
            blocked_commands: default_os_blocked_commands(),
            allowed_commands: Vec::new(),
            max_file_size_bytes: default_os_max_file_size_bytes(),
            shell_timeout_secs: default_os_shell_timeout_secs(),
            max_output_bytes: default_os_max_output_bytes(),
            max_list_entries: default_os_max_list_entries(),
            sensitive_env_patterns: default_os_sensitive_env_patterns(),
            sudo: SudoConfig::default(),
        }
    }
}

/// Privileged command execution via sudo.
#[derive(Debug, Clone, Deserialize)]
pub struct SudoConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub allowed_commands: Vec<String>,
    #[serde(default = "default_sudo_require_confirmation")]
    pub require_confirmation: bool,
    #[serde(default = "default_sudo_confirmation_timeout_secs")]
    pub confirmation_timeout_secs: u64,
    #[serde(default = "default_sudo_path")]
    pub sudo_path: String,
}

impl Default for SudoConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_commands: Vec::new(),
            require_confirmation: default_sudo_require_confirmation(),
            confirmation_timeout_secs: default_sudo_confirmation_timeout_secs(),
            sudo_path: default_sudo_path(),
        }
    }
}

fn default_sudo_require_confirmation() -> bool {
    true
}

fn default_sudo_confirmation_timeout_secs() -> u64 {
    120
}

fn default_sudo_path() -> String {
    "/usr/bin/sudo".into()
}

fn default_os_permission_level() -> String {
    "read-only".into()
}

fn default_os_allowed_paths() -> Vec<String> {
    vec!["/home".into(), "/tmp".into()]
}

fn default_os_blocked_paths() -> Vec<String> {
    vec![
        "/etc/shadow".into(),
        "/etc/gshadow".into(),
        "~/.ssh/id_*".into(),
    ]
}

fn default_os_blocked_commands() -> Vec<String> {
    vec![
        "rm -rf /".into(),
        "dd".into(),
        "mkfs".into(),
        "fdisk".into(),
    ]
}

fn default_os_max_file_size_bytes() -> u64 {
    10 * 1024 * 1024 // 10 MB
}

fn default_os_shell_timeout_secs() -> u64 {
    30
}

fn default_os_max_output_bytes() -> usize {
    1024 * 1024 // 1 MB
}

fn default_os_max_list_entries() -> usize {
    1000
}

fn default_os_sensitive_env_patterns() -> Vec<String> {
    vec![
        "*_KEY".into(),
        "*_SECRET".into(),
        "*_TOKEN".into(),
        "*_PASSWORD".into(),
    ]
}

/// Knowledge & RAG configuration.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct KnowledgeConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub vector_store: VectorStoreConfig,
    #[serde(default)]
    pub embeddings: EmbeddingsConfig,
    #[serde(default)]
    pub chunking: ChunkingConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VectorStoreConfig {
    #[serde(default = "default_vector_store_provider")]
    pub provider: String,
    #[serde(default = "default_vector_store_url")]
    pub url: String,
    #[serde(default = "default_collection_prefix")]
    pub collection_prefix: String,
    #[serde(default = "default_collection_name")]
    pub default_collection: String,
}

impl Default for VectorStoreConfig {
    fn default() -> Self {
        Self {
            provider: default_vector_store_provider(),
            url: default_vector_store_url(),
            collection_prefix: default_collection_prefix(),
            default_collection: default_collection_name(),
        }
    }
}

fn default_vector_store_provider() -> String {
    "qdrant".into()
}

fn default_vector_store_url() -> String {
    "http://localhost:6334".into()
}

fn default_collection_prefix() -> String {
    "orka_".into()
}

fn default_collection_name() -> String {
    "default".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct EmbeddingsConfig {
    #[serde(default = "default_embedding_provider")]
    pub provider: String,
    #[serde(default = "default_embedding_model")]
    pub model: String,
    #[serde(default = "default_embedding_dimensions")]
    pub dimensions: u32,
}

impl Default for EmbeddingsConfig {
    fn default() -> Self {
        Self {
            provider: default_embedding_provider(),
            model: default_embedding_model(),
            dimensions: default_embedding_dimensions(),
        }
    }
}

fn default_embedding_provider() -> String {
    "local".into()
}

fn default_embedding_model() -> String {
    "BAAI/bge-small-en-v1.5".into()
}

fn default_embedding_dimensions() -> u32 {
    384
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChunkingConfig {
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap: usize,
}

impl Default for ChunkingConfig {
    fn default() -> Self {
        Self {
            chunk_size: default_chunk_size(),
            chunk_overlap: default_chunk_overlap(),
        }
    }
}

fn default_chunk_size() -> usize {
    1000
}

fn default_chunk_overlap() -> usize {
    200
}

/// Scheduler configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct SchedulerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_scheduler_max_concurrent")]
    pub max_concurrent: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            poll_interval_secs: default_poll_interval_secs(),
            max_concurrent: default_scheduler_max_concurrent(),
        }
    }
}

fn default_poll_interval_secs() -> u64 {
    5
}

fn default_scheduler_max_concurrent() -> usize {
    4
}

/// HTTP client and webhook configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct HttpClientConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_http_max_response_bytes")]
    pub max_response_bytes: usize,
    #[serde(default = "default_http_timeout_secs")]
    pub default_timeout_secs: u64,
    #[serde(default = "default_http_blocked_domains")]
    pub blocked_domains: Vec<String>,
    #[serde(default = "default_http_user_agent")]
    pub user_agent: String,
    #[serde(default)]
    pub webhooks: WebhookConfig,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_response_bytes: default_http_max_response_bytes(),
            default_timeout_secs: default_http_timeout_secs(),
            blocked_domains: default_http_blocked_domains(),
            user_agent: default_http_user_agent(),
            webhooks: WebhookConfig::default(),
        }
    }
}

fn default_http_max_response_bytes() -> usize {
    1_048_576 // 1 MB
}

fn default_http_timeout_secs() -> u64 {
    30
}

fn default_http_blocked_domains() -> Vec<String> {
    vec!["169.254.169.254".into()]
}

fn default_http_user_agent() -> String {
    "Orka/0.1".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebhookConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_webhook_base_url")]
    pub base_url: String,
    #[serde(default = "default_webhook_path_prefix")]
    pub path_prefix: String,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: default_webhook_base_url(),
            path_prefix: default_webhook_path_prefix(),
        }
    }
}

fn default_webhook_base_url() -> String {
    "http://localhost:8080".into()
}

fn default_webhook_path_prefix() -> String {
    "/webhooks".into()
}

/// Experience & self-learning configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ExperienceConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Maximum number of principles to inject into the system prompt.
    #[serde(default = "default_experience_max_principles")]
    pub max_principles: usize,
    /// Minimum relevance score (0.0–1.0) for a principle to be injected.
    #[serde(default = "default_experience_min_relevance")]
    pub min_relevance_score: f32,
    /// When to trigger reflection: "failures", "all", or "sampled".
    #[serde(default = "default_experience_reflect_on")]
    pub reflect_on: String,
    /// Sampling rate for reflection when reflect_on = "sampled" (0.0–1.0).
    #[serde(default = "default_experience_sample_rate")]
    pub sample_rate: f64,
    /// Qdrant collection name for principles.
    #[serde(default = "default_experience_principles_collection")]
    pub principles_collection: String,
    /// Qdrant collection name for raw trajectories.
    #[serde(default = "default_experience_trajectories_collection")]
    pub trajectories_collection: String,
    /// LLM model override for reflection calls (uses default if unset).
    #[serde(default)]
    pub reflection_model: Option<String>,
    /// Maximum tokens for the reflection LLM call.
    #[serde(default = "default_experience_reflection_max_tokens")]
    pub reflection_max_tokens: u32,
    /// Number of trajectories to load per offline distillation run.
    #[serde(default = "default_experience_distillation_batch_size")]
    pub distillation_batch_size: usize,
    /// Similarity threshold for principle deduplication (0.0–1.0).
    #[serde(default = "default_experience_dedup_threshold")]
    pub dedup_threshold: f32,
    /// How often to run offline distillation, in seconds (0 = disabled).
    #[serde(default = "default_experience_distillation_interval_secs")]
    pub distillation_interval_secs: u64,
}

impl Default for ExperienceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_principles: default_experience_max_principles(),
            min_relevance_score: default_experience_min_relevance(),
            reflect_on: default_experience_reflect_on(),
            sample_rate: default_experience_sample_rate(),
            principles_collection: default_experience_principles_collection(),
            trajectories_collection: default_experience_trajectories_collection(),
            reflection_model: None,
            reflection_max_tokens: default_experience_reflection_max_tokens(),
            distillation_batch_size: default_experience_distillation_batch_size(),
            dedup_threshold: default_experience_dedup_threshold(),
            distillation_interval_secs: default_experience_distillation_interval_secs(),
        }
    }
}

fn default_experience_max_principles() -> usize {
    5
}

fn default_experience_min_relevance() -> f32 {
    0.6
}

fn default_experience_reflect_on() -> String {
    "failures".into()
}

fn default_experience_sample_rate() -> f64 {
    0.1
}

fn default_experience_principles_collection() -> String {
    "orka_principles".into()
}

fn default_experience_trajectories_collection() -> String {
    "orka_trajectories".into()
}

fn default_experience_reflection_max_tokens() -> u32 {
    1024
}

fn default_experience_distillation_batch_size() -> usize {
    20
}

fn default_experience_dedup_threshold() -> f32 {
    0.85
}

fn default_experience_distillation_interval_secs() -> u64 {
    3600 // 1 hour
}

impl OrkaConfig {
    /// Validate the loaded configuration.
    ///
    pub fn validate(&mut self) -> crate::Result<()> {
        self.llm.apply_defaults();

        if self.server.port == 0 {
            return Err(crate::Error::Config(
                "server.port must be in range 1-65535".into(),
            ));
        }

        if let Some(ref custom) = self.adapters.custom
            && custom.port == 0
        {
            return Err(crate::Error::Config(
                "adapters.custom.port must be in range 1-65535".into(),
            ));
        }

        if !self.redis.url.starts_with("redis://") && !self.redis.url.starts_with("rediss://") {
            return Err(crate::Error::Config(format!(
                "redis.url must start with redis:// or rediss://, got: {}",
                self.redis.url
            )));
        }

        if self.worker.concurrency == 0 {
            return Err(crate::Error::Config(
                "worker.concurrency must be greater than 0".into(),
            ));
        }

        for p in &self.llm.providers {
            if matches!(p.provider.as_str(), "anthropic" | "openai")
                && p.api_key_secret.is_none()
                && p.api_key.is_none()
                && p.api_key_env.is_none()
            {
                return Err(crate::Error::Config(format!(
                    "llm.providers[{}].api_key_secret, api_key, or api_key_env required for provider '{}'",
                    p.name, p.provider,
                )));
            }
        }

        if !Path::new(&self.workspace_dir).is_dir() {
            return Err(crate::Error::Config(format!(
                "workspace_dir '{}' does not exist or is not a directory",
                self.workspace_dir
            )));
        }

        // --- Empty strings ---
        if self.server.host.is_empty() {
            return Err(crate::Error::Config("server.host must not be empty".into()));
        }
        if self.agent.id.is_empty() {
            return Err(crate::Error::Config("agent.id must not be empty".into()));
        }
        for p in &self.llm.providers {
            if p.name.is_empty() {
                return Err(crate::Error::Config(
                    "llm.providers[].name must not be empty".into(),
                ));
            }
        }

        // --- Enum-like values ---
        if !matches!(
            self.logging.level.to_ascii_lowercase().as_str(),
            "trace" | "debug" | "info" | "warn" | "error"
        ) {
            return Err(crate::Error::Config(format!(
                "logging.level must be one of trace/debug/info/warn/error, got: '{}'",
                self.logging.level
            )));
        }
        if !matches!(self.bus.backend.as_str(), "redis" | "nats" | "memory") {
            return Err(crate::Error::Config(format!(
                "bus.backend must be one of redis/nats/memory, got: '{}'",
                self.bus.backend
            )));
        }
        if !matches!(self.sandbox.backend.as_str(), "process" | "wasm") {
            return Err(crate::Error::Config(format!(
                "sandbox.backend must be one of process/wasm, got: '{}'",
                self.sandbox.backend
            )));
        }
        for p in &self.llm.providers {
            if !matches!(p.provider.as_str(), "anthropic" | "openai" | "ollama") {
                return Err(crate::Error::Config(format!(
                    "llm.providers[{}].provider must be one of anthropic/openai/ollama, got: '{}'",
                    p.name, p.provider
                )));
            }
        }

        // --- Numeric upper bounds ---
        if self.worker.concurrency > 1024 {
            return Err(crate::Error::Config(format!(
                "worker.concurrency must be <= 1024, got: {}",
                self.worker.concurrency
            )));
        }
        if self.scheduler.max_concurrent > 1024 {
            return Err(crate::Error::Config(format!(
                "scheduler.max_concurrent must be <= 1024, got: {}",
                self.scheduler.max_concurrent
            )));
        }
        if self.llm.max_tokens == 0 {
            return Err(crate::Error::Config(
                "llm.max_tokens must be greater than 0".into(),
            ));
        }
        for p in &self.llm.providers {
            if p.max_tokens == Some(0) {
                return Err(crate::Error::Config(format!(
                    "llm.providers[{}].max_tokens must be greater than 0",
                    p.name
                )));
            }
        }

        // --- Timeouts > 0 ---
        if self.llm.timeout_secs == 0 {
            return Err(crate::Error::Config(
                "llm.timeout_secs must be greater than 0".into(),
            ));
        }
        if self.sandbox.limits.timeout_secs == 0 {
            return Err(crate::Error::Config(
                "sandbox.limits.timeout_secs must be greater than 0".into(),
            ));
        }
        if self.http.default_timeout_secs == 0 {
            return Err(crate::Error::Config(
                "http.default_timeout_secs must be greater than 0".into(),
            ));
        }

        // --- Float ranges ---
        if !(0.0..=1.0).contains(&self.experience.min_relevance_score) {
            return Err(crate::Error::Config(format!(
                "experience.min_relevance must be in 0.0..=1.0, got: {}",
                self.experience.min_relevance_score
            )));
        }
        if !(0.0..=1.0).contains(&self.experience.sample_rate) {
            return Err(crate::Error::Config(format!(
                "experience.sample_rate must be in 0.0..=1.0, got: {}",
                self.experience.sample_rate
            )));
        }

        // --- Cross-field invariants ---
        if self.knowledge.chunking.chunk_overlap >= self.knowledge.chunking.chunk_size {
            return Err(crate::Error::Config(format!(
                "knowledge.chunking.chunk_overlap ({}) must be less than chunk_size ({})",
                self.knowledge.chunking.chunk_overlap, self.knowledge.chunking.chunk_size
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
            if let Some(ref default) = self.default_workspace
                && !self.workspaces.iter().any(|w| &w.name == default)
            {
                return Err(crate::Error::Config(format!(
                    "default_workspace '{}' not found in [[workspaces]]",
                    default
                )));
            }
        }

        Ok(())
    }

    /// Resolve the config file path.
    ///
    /// Resolution order:
    /// 1. Explicit `path` argument
    /// 2. `ORKA_CONFIG` environment variable
    /// 3. `orka.toml` in the current directory
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
    ///
    /// The raw TOML is read, migrated in-memory if needed (preserving
    /// comments), and then deserialized. Environment variable overlays
    /// are applied on top.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let config_path = Self::resolve_path(path);

        let mut builder = Config::builder();

        if config_path.exists() {
            // Read raw TOML so we can run migrations before deserializing.
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

        builder.build()?.try_deserialize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let cfg = OrkaConfig::load(None).unwrap_or_else(|_| OrkaConfig {
            config_version: 1,
            server: ServerConfig::default(),
            bus: BusConfig::default(),
            redis: RedisConfig::default(),
            logging: LoggingConfig::default(),
            workspace_dir: default_workspace_dir(),
            workspaces: Vec::new(),
            default_workspace: None,
            adapters: AdapterConfig::default(),
            worker: WorkerConfig::default(),
            memory: MemoryConfig::default(),
            secrets: SecretConfig::default(),
            auth: AuthConfig::default(),
            sandbox: SandboxConfig::default(),
            plugins: PluginConfig::default(),
            session: SessionConfig::default(),
            queue: QueueConfig::default(),
            llm: LlmConfig::default(),
            agent: AgentConfig::default(),
            tools: ToolsConfig::default(),
            observe: ObserveConfig::default(),
            gateway: GatewayConfig::default(),
            mcp: McpConfig::default(),
            guardrails: GuardrailsConfig::default(),
            web: WebConfig::default(),
            os: OsConfig::default(),
            a2a: A2aConfig::default(),
            knowledge: KnowledgeConfig::default(),
            scheduler: SchedulerConfig::default(),
            http: HttpClientConfig::default(),
            experience: ExperienceConfig::default(),
        });
        assert_eq!(cfg.server.port, 8080);
        assert_eq!(cfg.bus.backend, "redis");
        assert!(!cfg.auth.enabled);
        assert_eq!(cfg.sandbox.backend, "process");
    }

    fn valid_config() -> OrkaConfig {
        OrkaConfig {
            config_version: 1,
            server: ServerConfig::default(),
            bus: BusConfig::default(),
            redis: RedisConfig::default(),
            logging: LoggingConfig::default(),
            workspace_dir: ".".into(), // current dir exists
            workspaces: Vec::new(),
            default_workspace: None,
            adapters: AdapterConfig::default(),
            worker: WorkerConfig::default(),
            memory: MemoryConfig::default(),
            secrets: SecretConfig::default(),
            auth: AuthConfig::default(),
            sandbox: SandboxConfig::default(),
            plugins: PluginConfig::default(),
            session: SessionConfig::default(),
            queue: QueueConfig::default(),
            llm: LlmConfig::default(),
            agent: AgentConfig::default(),
            tools: ToolsConfig::default(),
            observe: ObserveConfig::default(),
            gateway: GatewayConfig::default(),
            mcp: McpConfig::default(),
            guardrails: GuardrailsConfig::default(),
            web: WebConfig::default(),
            os: OsConfig::default(),
            a2a: A2aConfig::default(),
            knowledge: KnowledgeConfig::default(),
            scheduler: SchedulerConfig::default(),
            http: HttpClientConfig::default(),
            experience: ExperienceConfig::default(),
        }
    }

    #[test]
    fn valid_config_passes() {
        let mut cfg = valid_config();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn port_zero_rejected() {
        let mut cfg = valid_config();
        cfg.server.port = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn invalid_redis_url_rejected() {
        let mut cfg = valid_config();
        cfg.redis.url = "http://localhost:6379".into();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn concurrency_zero_rejected() {
        let mut cfg = valid_config();
        cfg.worker.concurrency = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn workspace_dir_nonexistent_rejected() {
        let mut cfg = valid_config();
        cfg.workspace_dir = "/nonexistent/path".into();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn sudo_config_defaults() {
        let sudo = SudoConfig::default();
        assert!(!sudo.enabled);
        assert!(sudo.allowed_commands.is_empty());
        assert!(sudo.require_confirmation);
        assert_eq!(sudo.confirmation_timeout_secs, 120);
        assert_eq!(sudo.sudo_path, "/usr/bin/sudo");
    }

    #[test]
    fn os_config_has_sudo_default() {
        let os = OsConfig::default();
        assert!(!os.sudo.enabled);
    }

    #[test]
    fn apply_defaults_propagates_llm_timeout() {
        let mut llm = LlmConfig {
            timeout_secs: 120,
            max_tokens: 4096,
            max_retries: 5,
            providers: vec![LlmProviderConfig {
                name: "test".into(),
                provider: "anthropic".into(),
                api_key_secret: Some("k".into()),
                api_key: None,
                api_key_env: None,
                model: "test-model".into(),
                timeout_secs: None,
                max_tokens: None,
                max_retries: Some(1), // explicitly set
                base_url: None,
                prefixes: Vec::new(),
                cost_per_1k_input_tokens: None,
                cost_per_1k_output_tokens: None,
            }],
            ..LlmConfig::default()
        };
        llm.apply_defaults();
        assert_eq!(llm.providers[0].timeout_secs, Some(120)); // inherited
        assert_eq!(llm.providers[0].max_tokens, Some(4096)); // inherited
        assert_eq!(llm.providers[0].max_retries, Some(1)); // NOT overwritten
    }
}
