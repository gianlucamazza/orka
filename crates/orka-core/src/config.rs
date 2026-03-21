use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use std::fmt;
use std::path::Path;

use crate::migrate;

/// A named workspace entry for multi-workspace support.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceEntry {
    /// Unique name for this workspace, used for routing and CLI selection.
    pub name: String,
    /// Filesystem path to the workspace directory.
    pub dir: String,
}

/// Top-level Orka configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct OrkaConfig {
    /// Config schema version. `0` = legacy/absent; current version = `3`.
    #[serde(default)]
    pub config_version: u32,
    /// HTTP server bind configuration.
    #[serde(default = "default_server")]
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
    #[serde(default = "default_workspace_dir")]
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
    /// Linux OS integration configuration.
    #[serde(default)]
    pub os: OsConfig,
    /// Agent-to-Agent (A2A) protocol configuration.
    #[serde(default)]
    pub a2a: A2aConfig,
    /// Knowledge base and RAG configuration.
    #[serde(default)]
    pub knowledge: KnowledgeConfig,
    /// Cron scheduler configuration.
    #[serde(default)]
    pub scheduler: SchedulerConfig,
    /// HTTP client and webhook configuration.
    #[serde(default)]
    pub http: HttpClientConfig,
    /// Experience / self-learning configuration.
    #[serde(default)]
    pub experience: ExperienceConfig,
    /// Multi-agent definitions (replaces single `[agent]` for multi-agent deployments).
    #[serde(default)]
    pub agents: Vec<AgentDef>,
    /// Graph topology for multi-agent execution.
    #[serde(default)]
    pub graph: Option<GraphDef>,
}

/// Web search and read configuration.
#[derive(Clone, Deserialize)]
pub struct WebConfig {
    /// Search backend to use (`"tavily"`, `"brave"`, `"searxng"`, or `"none"`).
    #[serde(default = "default_web_search_provider")]
    pub search_provider: String,
    /// Direct API key for the search provider (prefer `api_key_env` in production).
    #[serde(default)]
    pub api_key: Option<String>,
    /// Environment variable name containing the search provider API key.
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// Base URL for a SearXNG instance (required when `search_provider = "searxng"`).
    #[serde(default)]
    pub searxng_base_url: Option<String>,
    /// Maximum number of search results to return per query.
    #[serde(default = "default_web_max_results")]
    pub max_results: usize,
    /// Maximum characters to read from a single web page.
    #[serde(default = "default_web_max_read_chars")]
    pub max_read_chars: usize,
    /// Maximum characters of extracted content to include in a skill result.
    #[serde(default = "default_web_max_content_chars")]
    pub max_content_chars: usize,
    /// Time-to-live in seconds for cached search results.
    #[serde(default = "default_web_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
    /// Timeout in seconds for HTTP read requests.
    #[serde(default = "default_web_read_timeout_secs")]
    pub read_timeout_secs: u64,
    /// User-Agent header sent with web requests.
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
    format!("Orka/{} (Web Agent)", env!("CARGO_PKG_VERSION"))
}

/// HTTP server bind configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// IP address or hostname to bind on.
    #[serde(default = "default_host")]
    pub host: String,
    /// TCP port to listen on.
    #[serde(default = "default_port")]
    pub port: u16,
}

/// Message bus configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct BusConfig {
    /// Bus backend to use (`"redis"`, `"nats"`, or `"memory"`).
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

/// Redis connection configuration.
#[non_exhaustive]
#[derive(Clone, Deserialize)]
pub struct RedisConfig {
    /// Redis connection URL (e.g. `"redis://127.0.0.1:6379"`).
    #[serde(default = "default_redis_url")]
    pub url: String,
}

/// Structured logging configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    /// Log level filter (`"trace"`, `"debug"`, `"info"`, `"warn"`, or `"error"`).
    #[serde(default = "default_log_level")]
    pub level: String,
    /// Emit logs as JSON (useful for log aggregators).
    #[serde(default)]
    pub json: bool,
}

/// Channel adapter configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AdapterConfig {
    /// Custom HTTP adapter configuration.
    pub custom: Option<CustomAdapterConfig>,
    /// Telegram bot adapter configuration.
    pub telegram: Option<TelegramAdapterConfig>,
    /// Discord bot adapter configuration.
    pub discord: Option<DiscordAdapterConfig>,
    /// Slack bot adapter configuration.
    pub slack: Option<SlackAdapterConfig>,
    /// WhatsApp Cloud API adapter configuration.
    pub whatsapp: Option<WhatsAppAdapterConfig>,
}

/// Telegram bot adapter configuration.
#[derive(Clone, Default, Deserialize)]
pub struct TelegramAdapterConfig {
    /// Secret store path for the Telegram bot token.
    pub bot_token_secret: Option<String>,
    /// Workspace name to route messages to (uses default if unset).
    #[serde(default)]
    pub workspace: Option<String>,
    /// Receive mode: "polling" (default) or "webhook".
    #[serde(default)]
    pub mode: Option<String>,
    /// Public HTTPS URL for webhook mode (e.g. `https://example.com/telegram/webhook`).
    #[serde(default)]
    pub webhook_url: Option<String>,
    /// Local port to listen on in webhook mode (default 8443).
    #[serde(default)]
    pub webhook_port: Option<u16>,
    /// Outbound text parse mode: "HTML" (default), "MarkdownV2", or "none".
    #[serde(default)]
    pub parse_mode: Option<String>,
    /// Enable streaming via editMessageText (default false).
    #[serde(default)]
    pub streaming: Option<bool>,
    /// Telegram user ID of the bot owner; if set, only this user (and `allowed_users`) may interact.
    #[serde(default)]
    pub owner_id: Option<i64>,
    /// Additional Telegram user IDs allowed to interact with the bot.
    #[serde(default)]
    pub allowed_users: Option<Vec<i64>>,
    /// How to handle messages in group chats: "all" (default) processes every message,
    /// "commands_only" ignores non-command messages in groups/supergroups.
    #[serde(default)]
    pub group_mode: Option<String>,
}

/// Discord bot adapter configuration.
#[non_exhaustive]
#[derive(Clone, Default, Deserialize)]
pub struct DiscordAdapterConfig {
    /// Secret store path for the Discord bot token.
    pub bot_token_secret: Option<String>,
    /// Discord application ID (required for slash command registration).
    #[serde(default)]
    pub application_id: Option<String>,
    /// Workspace name to route messages to (uses default if unset).
    #[serde(default)]
    pub workspace: Option<String>,
}

/// Slack bot adapter configuration.
#[non_exhaustive]
#[derive(Clone, Deserialize)]
pub struct SlackAdapterConfig {
    /// Secret store path for the Slack bot token.
    pub bot_token_secret: Option<String>,
    /// Local port to listen on for Slack event payloads.
    #[serde(default = "default_slack_port")]
    pub listen_port: u16,
    /// Workspace name to route messages to (uses default if unset).
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

/// WhatsApp Cloud API adapter configuration.
#[non_exhaustive]
#[derive(Clone, Deserialize)]
pub struct WhatsAppAdapterConfig {
    /// Secret store path for the WhatsApp access token.
    pub access_token_secret: Option<String>,
    /// WhatsApp Cloud API phone number ID.
    pub phone_number_id: Option<String>,
    /// Secret store path for the webhook verify token.
    pub verify_token_secret: Option<String>,
    /// Local port to listen on for incoming webhook events.
    #[serde(default = "default_whatsapp_port")]
    pub listen_port: u16,
    /// Workspace name to route messages to (uses default if unset).
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

/// Custom HTTP adapter configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct CustomAdapterConfig {
    /// IP address or hostname for the custom adapter to bind on.
    #[serde(default = "default_custom_host")]
    pub host: String,
    /// TCP port for the custom adapter to listen on.
    #[serde(default = "default_custom_port")]
    pub port: u16,
    /// Workspace name to route messages to (uses default if unset).
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

/// Worker pool configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct WorkerConfig {
    /// Number of concurrent worker tasks to spawn.
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    /// Base delay in milliseconds for exponential retry backoff.
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

/// In-memory store (Redis) configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct MemoryConfig {
    /// Maximum number of entries to store before oldest entries are evicted.
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
    /// Backend to use (`"redis"`, `"memory"`, or `"auto"` to follow `bus.backend`).
    #[serde(default = "default_backend_auto")]
    pub backend: String,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            max_entries: default_max_entries(),
            backend: default_backend_auto(),
        }
    }
}

fn default_max_entries() -> usize {
    10_000
}

fn default_backend_auto() -> String {
    "auto".into()
}

/// Secret storage configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SecretConfig {
    /// Environment variable name containing the 32-byte hex-encoded encryption key
    /// for encrypting secrets at rest in Redis. If unset or the env var is missing,
    /// secrets are stored in plaintext (development mode).
    #[serde(default)]
    pub encryption_key_env: Option<String>,
}

/// HTTP authentication configuration.
#[derive(Clone, Deserialize)]
pub struct AuthConfig {
    /// Whether authentication is required on incoming requests.
    #[serde(default)]
    pub enabled: bool,
    /// HTTP header name that carries API key credentials.
    #[serde(default = "default_api_key_header")]
    pub api_key_header: String,
    /// Static API key entries (hashed).
    #[serde(default)]
    pub api_keys: Vec<ApiKeyEntry>,
    /// JWT authentication configuration (optional, mutually exclusive with API keys).
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

/// JWT authentication configuration.
#[non_exhaustive]
#[derive(Clone, Deserialize)]
pub struct JwtAuthConfig {
    /// Expected `iss` claim value.
    pub issuer: String,
    /// Expected `aud` claim value.
    #[serde(default)]
    pub audience: Option<String>,
    /// URL of the JWKS endpoint for RS256 verification.
    pub jwks_uri: Option<String>,
    /// Static secret for HS256 (alternative to JWKS).
    pub secret: Option<String>,
}

fn default_api_key_header() -> String {
    "X-Api-Key".into()
}

/// A single API key entry with a hashed key and optional scopes.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiKeyEntry {
    /// Human-readable label for this key (e.g. the client name).
    pub name: String,
    /// Bcrypt or SHA-256 hash of the raw API key.
    pub key_hash: String,
    /// Optional permission scopes granted to this key.
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// Code sandbox configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct SandboxConfig {
    /// Sandbox backend to use (`"process"` or `"wasm"`).
    #[serde(default = "default_sandbox_backend")]
    pub backend: String,
    /// Resource limits applied to sandbox executions.
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

/// Resource limits applied to sandbox executions.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct SandboxLimitsConfig {
    /// Maximum wall-clock time in seconds before the sandbox is killed.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Maximum memory in bytes the sandbox process may use.
    #[serde(default = "default_max_memory_bytes")]
    pub max_memory_bytes: usize,
    /// Maximum combined stdout + stderr size in bytes.
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

/// Sandbox capabilities granted to a specific WASM plugin (deny-by-default).
#[non_exhaustive]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PluginCapabilities {
    /// Allowed network hosts in `host:port` format.
    #[serde(default)]
    pub network: Vec<String>,
    /// Allowed filesystem paths (pre-opened directories).
    #[serde(default)]
    pub fs: Vec<String>,
    /// Allowed environment variable names.
    #[serde(default)]
    pub env: Vec<String>,
}

/// WASM plugin configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PluginConfig {
    /// Directory to scan for `.wasm` plugin files.
    pub dir: Option<String>,
    /// Per-plugin capability overrides keyed by plugin name.
    #[serde(default)]
    pub capabilities: std::collections::HashMap<String, PluginCapabilities>,
}

/// Soft skill (SKILL.md) configuration.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SoftSkillConfig {
    /// Directory to scan for soft skill subdirectories containing SKILL.md files.
    pub dir: Option<String>,
}

/// Session store configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct SessionConfig {
    /// Session time-to-live in seconds (default: 86400 = 24 hours).
    #[serde(default = "default_session_ttl_secs")]
    pub ttl_secs: u64,
    /// Backend to use (`"redis"`, `"memory"`, or `"auto"` to follow `bus.backend`).
    #[serde(default = "default_backend_auto")]
    pub backend: String,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            ttl_secs: default_session_ttl_secs(),
            backend: default_backend_auto(),
        }
    }
}

fn default_session_ttl_secs() -> u64 {
    86400 // 24 hours
}

/// Priority queue configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct QueueConfig {
    /// Maximum number of handler retries before a message is moved to the dead-letter queue.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Backend to use (`"redis"`, `"memory"`, or `"auto"` to follow `bus.backend`).
    #[serde(default = "default_backend_auto")]
    pub backend: String,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
            backend: default_backend_auto(),
        }
    }
}

fn default_max_retries() -> u32 {
    3
}

/// Skill invocation audit log configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct AuditConfig {
    /// Enable audit logging of skill invocations (default: false).
    #[serde(default)]
    pub enabled: bool,
    /// Output backend: `"file"` (JSONL) or `"redis"` (stream). Default: `"file"`.
    #[serde(default = "default_audit_output")]
    pub output: String,
    /// Path for file-based audit log (default: `"orka-audit.jsonl"`).
    #[serde(default = "default_audit_path")]
    pub path: Option<String>,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            output: default_audit_output(),
            path: default_audit_path(),
        }
    }
}

fn default_audit_output() -> String {
    "file".into()
}

fn default_audit_path() -> Option<String> {
    Some("orka-audit.jsonl".into())
}

/// Observability (metrics/tracing) configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct ObserveConfig {
    /// Backend to emit events to (`"log"`, `"redis"`, or `"otel"`).
    #[serde(default = "default_observe_backend")]
    pub backend: String,
    /// Number of events to batch before flushing.
    #[serde(default = "default_observe_batch_size")]
    pub batch_size: usize,
    /// Maximum interval in milliseconds between flushes.
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

/// API gateway rate limiting and deduplication configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    /// Maximum requests per minute per session before rate limiting kicks in.
    #[serde(default = "default_gateway_rate_limit")]
    pub rate_limit: u32,
    /// Time-to-live in seconds for deduplication entries.
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

/// LLM provider configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    /// Default model name used when no provider matches.
    #[serde(default = "default_llm_model")]
    pub model: String,
    /// Default request timeout in seconds.
    #[serde(default = "default_llm_timeout_secs")]
    pub timeout_secs: u64,
    /// Default maximum output tokens per request.
    #[serde(default = "default_llm_max_tokens")]
    pub max_tokens: u32,
    /// Default maximum number of retries on transient errors.
    #[serde(default = "default_llm_max_retries")]
    pub max_retries: u32,
    /// Anthropic API version header value (e.g. `"2023-06-01"`).
    #[serde(default = "default_llm_api_version")]
    pub api_version: String,
    /// Named provider configurations for multi-provider routing.
    #[serde(default)]
    pub providers: Vec<LlmProviderConfig>,
    /// Total context window size in tokens (used for history truncation).
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

/// Configuration for a single named LLM provider.
#[non_exhaustive]
#[derive(Clone, Deserialize)]
pub struct LlmProviderConfig {
    /// Unique identifier for this provider entry (e.g. `"anthropic-prod"`).
    pub name: String,
    /// Provider type: `"anthropic"`, `"openai"`, or `"ollama"`.
    pub provider: String,
    /// Secret store path for the API key.
    #[serde(default)]
    pub api_key_secret: Option<String>,
    /// Direct API key (not recommended for production — use secrets store instead).
    #[serde(default)]
    pub api_key: Option<String>,
    /// Environment variable name for the API key (e.g. "ANTHROPIC_API_KEY").
    /// Checked before the secret store. If set and the env var exists, skip secret store lookup.
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// Default model name for this provider.
    #[serde(default = "default_llm_model")]
    pub model: String,
    /// Request timeout in seconds (inherits from `llm.timeout_secs` if unset).
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// Maximum output tokens (inherits from `llm.max_tokens` if unset).
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Maximum retries (inherits from `llm.max_retries` if unset).
    #[serde(default)]
    pub max_retries: Option<u32>,
    /// Override the API base URL (useful for proxies or local models).
    #[serde(default)]
    pub base_url: Option<String>,
    /// Model name prefixes this provider handles (e.g. `["claude"]`).
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
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    /// Unique identifier for this agent.
    #[serde(default = "default_agent_id")]
    pub id: String,
    /// Human-readable display name shown in status commands.
    #[serde(default = "default_agent_display_name")]
    pub display_name: String,
    /// IANA timezone name for time-aware operations (e.g. `"America/New_York"`).
    #[serde(default)]
    pub timezone: Option<String>,
    /// Maximum tool-loop iterations before the agent gives up.
    #[serde(default = "default_agent_max_iterations")]
    pub max_iterations: usize,
    /// Override the LLM model for this agent.
    #[serde(default)]
    pub model: Option<String>,
    /// Override the maximum output tokens for this agent.
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Override the context window size for history truncation.
    #[serde(default)]
    pub context_window_tokens: Option<u32>,
    /// Cumulative token budget per session before the session is closed.
    #[serde(default)]
    pub max_tokens_per_session: Option<u64>,
    /// Interval in seconds at which the agent sends a heartbeat event.
    #[serde(default)]
    pub heartbeat_interval_secs: Option<u64>,
    /// LLM model used for history summarization (uses `model` if unset).
    #[serde(default)]
    pub summarization_model: Option<String>,
    /// Number of history tokens that triggers automatic summarization.
    #[serde(default)]
    pub summarization_threshold: Option<usize>,
    /// Maximum number of *messages* (not turns) to retain in the conversation history.
    ///
    /// **Note:** This counts individual messages, not conversation turns.
    /// A single tool-use turn typically produces 3+ messages (user → assistant
    /// with tool_use block → user with tool_result block).  Set this value at
    /// least 3× the number of logical turns you want to retain.
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
    /// Sampling temperature (0.0–2.0). Note: Anthropic forces temperature=1 when thinking is enabled.
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Token budget for Anthropic extended thinking. Mutually exclusive with `reasoning_effort`.
    #[serde(default)]
    pub thinking_budget_tokens: Option<u32>,
    /// Reasoning effort for OpenAI o-series models (`"low"`, `"medium"`, `"high"`).
    /// Mutually exclusive with `thinking_budget_tokens`.
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    /// Maximum output size in bytes for a single skill invocation (default: no limit).
    ///
    /// Prevents oversized skill outputs from flooding the LLM context window.
    #[serde(default)]
    pub skill_max_output_bytes: Option<usize>,
    /// Maximum execution time per skill invocation in milliseconds (default: no limit).
    ///
    /// Post-hoc budget check on top of the hard `skill_timeout_secs` cancellation.
    #[serde(default)]
    pub skill_max_duration_ms: Option<u64>,
    /// Enable progressive tool disclosure.
    ///
    /// When `true`, the agent starts with only two synthetic tools (`list_tool_categories`
    /// and `enable_tools`) and must explicitly enable skill categories before using them.
    /// This reduces initial context window usage.
    #[serde(default)]
    pub progressive_disclosure: bool,
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
            temperature: None,
            thinking_budget_tokens: None,
            reasoning_effort: None,
            skill_max_output_bytes: None,
            skill_max_duration_ms: None,
            progressive_disclosure: false,
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

/// Definition for a single agent in a multi-agent deployment.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct AgentDef {
    /// Unique agent identifier.
    pub id: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Path to the agent's soul/system-prompt file.
    #[serde(default)]
    pub soul_file: Option<String>,
    /// Inline soul/system-prompt text (overrides `soul_file` if both set).
    #[serde(default)]
    pub soul: Option<String>,
    /// Path to the agent's tools configuration file.
    #[serde(default)]
    pub tools_file: Option<String>,
    /// LLM model override (uses global default if unset).
    #[serde(default)]
    pub model: Option<String>,
    /// Maximum agentic loop iterations.
    #[serde(default)]
    pub max_iterations: Option<usize>,
    /// Maximum output tokens per LLM call.
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Context window size override.
    #[serde(default)]
    pub context_window: Option<u32>,
    /// Agent IDs this agent may hand off to.
    #[serde(default)]
    pub handoff_targets: Vec<String>,
    /// Tool allow/deny scope for this agent.
    #[serde(default)]
    pub tools: Option<ToolScopeDef>,
    /// Sampling temperature (0.0–2.0).
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Token budget for Anthropic extended thinking. Mutually exclusive with `reasoning_effort`.
    #[serde(default)]
    pub thinking_budget_tokens: Option<u32>,
    /// Reasoning effort for OpenAI o-series models (`"low"`, `"medium"`, `"high"`).
    /// Mutually exclusive with `thinking_budget_tokens`.
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

/// Tool scope definition: allow-list or deny-list.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ToolScopeDef {
    /// Only the listed tools are available.
    Allow {
        /// Names of allowed tools.
        allow: Vec<String>,
    },
    /// All tools except the listed ones are available.
    Deny {
        /// Names of denied tools.
        deny: Vec<String>,
    },
}

/// Graph topology definition.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct GraphDef {
    /// Optional graph identifier.
    #[serde(default)]
    pub id: Option<String>,
    /// ID of the entry-point agent.
    pub entry: String,
    /// IDs of terminal agents (execution stops here).
    #[serde(default)]
    pub terminal: Vec<String>,
    /// Maximum total iterations across all agents.
    #[serde(default)]
    pub max_total_iterations: Option<usize>,
    /// Maximum total tokens consumed across all agents.
    #[serde(default)]
    pub max_total_tokens: Option<u64>,
    /// Maximum wall-clock execution time in seconds.
    #[serde(default)]
    pub max_duration_secs: Option<u64>,
    /// Directed edges between agents.
    #[serde(default)]
    pub edges: Vec<EdgeDef>,
}

/// An edge definition in the graph.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct EdgeDef {
    /// Source agent ID.
    pub from: String,
    /// Destination agent ID.
    pub to: String,
    /// Optional condition that must hold for this edge to be taken.
    #[serde(default)]
    pub condition: Option<EdgeConditionDef>,
    /// Edge priority (higher = preferred when multiple edges are eligible).
    #[serde(default)]
    pub priority: Option<u32>,
}

impl AgentDef {
    /// Create a minimal agent definition with the given ID.
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            display_name: format!("{id} Agent"),
            id,
            soul_file: None,
            soul: None,
            tools_file: None,
            model: None,
            max_iterations: None,
            max_tokens: None,
            context_window: None,
            handoff_targets: vec![],
            tools: None,
            temperature: None,
            thinking_budget_tokens: None,
            reasoning_effort: None,
        }
    }
}

impl GraphDef {
    /// Create a graph definition with the given entry-point agent ID.
    pub fn new(entry: impl Into<String>) -> Self {
        Self {
            id: None,
            entry: entry.into(),
            terminal: vec![],
            max_total_iterations: None,
            max_total_tokens: None,
            max_duration_secs: None,
            edges: vec![],
        }
    }
}

impl EdgeDef {
    /// Create an edge from one agent to another.
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            condition: None,
            priority: None,
        }
    }
}

/// Edge condition definition.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum EdgeConditionDef {
    /// Take this edge when a state key matches a value.
    #[serde(rename = "state_match")]
    StateMatch {
        /// State key to test.
        key: String,
        /// Expected value.
        value: serde_json::Value,
    },
    /// Take this edge when the agent output contains a substring.
    #[serde(rename = "output_contains")]
    OutputContains {
        /// Substring to match in the agent output.
        pattern: String,
    },
    /// Always take this edge (unconditional).
    #[serde(rename = "always")]
    Always,
}

/// Tool enable/disable configuration.
#[non_exhaustive]
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

/// MCP (Model Context Protocol) client and server configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct McpConfig {
    /// MCP server processes to launch and connect to.
    #[serde(default)]
    pub servers: Vec<McpServerEntry>,
    /// Configuration for exposing this agent's skills as an MCP server.
    #[serde(default)]
    pub serve: Option<McpServeConfig>,
}

/// Configuration for exposing Orka skills as an MCP server.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct McpServeConfig {
    /// Whether the MCP server is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Transport to use: `"stdio"` or `"sse"`.
    #[serde(default = "default_mcp_serve_transport")]
    pub transport: String,
    /// TCP port for SSE transport (required when `transport = "sse"`).
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

/// OAuth 2.1 Client Credentials config for an MCP HTTP server.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct McpAuthEntry {
    /// Token endpoint URL.
    pub token_url: String,
    /// OAuth client ID.
    pub client_id: String,
    /// Name of the environment variable holding the client secret.
    pub client_secret_env: String,
    /// Scopes to request.
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// A single MCP server to connect to.
///
/// Exactly one of `command` (stdio) or `url` (streamable HTTP) must be set.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerEntry {
    /// Unique name for this MCP server (used to prefix tool names).
    pub name: String,
    /// Stdio transport: executable path or command to launch.
    #[serde(default)]
    pub command: Option<String>,
    /// Arguments to pass to the command (stdio transport only).
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables to inject into the process (stdio transport only).
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// Streamable HTTP transport: base URL of the MCP endpoint.
    #[serde(default)]
    pub url: Option<String>,
    /// OAuth 2.1 credentials for the HTTP transport.
    #[serde(default)]
    pub auth: Option<McpAuthEntry>,
}

/// Content guardrails configuration.
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

/// A regex redaction rule.
#[derive(Debug, Clone, Deserialize)]
pub struct RedactPattern {
    /// Regex pattern to match.
    pub pattern: String,
    /// Replacement string for matched text.
    pub replacement: String,
}

/// Agent-to-Agent (A2A) protocol configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct A2aConfig {
    /// Whether the A2A endpoint is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Public base URL of this agent for A2A discovery.
    #[serde(default)]
    pub url: Option<String>,
}

/// Linux OS integration configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct OsConfig {
    /// Whether OS integration skills are enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Permission level: `"read-only"`, `"interact"`, `"write"`, `"execute"`, or `"admin"`.
    #[serde(default = "default_os_permission_level")]
    pub permission_level: String,
    /// Filesystem paths the agent is permitted to access.
    #[serde(default = "default_os_allowed_paths")]
    pub allowed_paths: Vec<String>,
    /// Filesystem paths that are always denied, even if in `allowed_paths`.
    #[serde(default = "default_os_blocked_paths")]
    pub blocked_paths: Vec<String>,
    /// Shell command substrings that are never permitted.
    #[serde(default = "default_os_blocked_commands")]
    pub blocked_commands: Vec<String>,
    /// Explicit shell commands that are always permitted (empty = all allowed).
    #[serde(default)]
    pub allowed_commands: Vec<String>,
    /// Maximum file size in bytes the agent may read or write.
    #[serde(default = "default_os_max_file_size_bytes")]
    pub max_file_size_bytes: u64,
    /// Shell command timeout in seconds.
    #[serde(default = "default_os_shell_timeout_secs")]
    pub shell_timeout_secs: u64,
    /// Maximum combined stdout + stderr size in bytes from a shell command.
    #[serde(default = "default_os_max_output_bytes")]
    pub max_output_bytes: usize,
    /// Maximum number of entries returned by directory listing.
    #[serde(default = "default_os_max_list_entries")]
    pub max_list_entries: usize,
    /// Glob patterns for environment variables that must not be exposed.
    #[serde(default = "default_os_sensitive_env_patterns")]
    pub sensitive_env_patterns: Vec<String>,
    /// Privileged sudo configuration.
    #[serde(default)]
    pub sudo: SudoConfig,
    /// Claude Code delegation skill configuration.
    #[serde(default)]
    pub claude_code: ClaudeCodeConfig,
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
            claude_code: ClaudeCodeConfig::default(),
        }
    }
}

/// Configuration for the `claude_code` delegation skill.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeCodeConfig {
    /// Tri-state enable flag: `"auto"` (default) probes for `claude` on PATH,
    /// `"true"` forces registration, `"false"` disables unconditionally.
    #[serde(default = "default_claude_code_enabled")]
    pub enabled: String,
    /// Claude model to use (e.g. `"claude-sonnet-4-6"`). Uses Claude Code's default if unset.
    #[serde(default)]
    pub model: Option<String>,
    /// Maximum agentic turns for a single task (`--max-turns`).
    #[serde(default)]
    pub max_turns: Option<u32>,
    /// Execution timeout in seconds (default 300).
    #[serde(default = "default_claude_code_timeout_secs")]
    pub timeout_secs: u64,
    /// Working directory for the `claude` subprocess. Defaults to the process cwd.
    #[serde(default)]
    pub working_dir: Option<String>,
    /// Additional system-level instructions appended via `--append-system-prompt`.
    /// Use for project-specific conventions that Claude Code should always follow.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Tool allowlist passed via `--allowedTools` (e.g. `["Read", "Edit", "Bash(cargo *)"]`).
    /// An empty list means no restriction — Claude Code uses its default tool set.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// When `true` (default), inject workspace info (cwd, agent name) into the task prompt
    /// so Claude Code has project context without requiring the caller to supply it.
    #[serde(default = "default_true")]
    pub inject_context: bool,
}

impl Default for ClaudeCodeConfig {
    fn default() -> Self {
        Self {
            enabled: default_claude_code_enabled(),
            model: None,
            max_turns: None,
            timeout_secs: default_claude_code_timeout_secs(),
            working_dir: None,
            system_prompt: None,
            allowed_tools: Vec::new(),
            inject_context: true,
        }
    }
}

fn default_claude_code_enabled() -> String {
    "auto".into()
}

fn default_claude_code_timeout_secs() -> u64 {
    300
}

fn default_true() -> bool {
    true
}

/// Privileged command execution via sudo.
#[derive(Debug, Clone, Deserialize)]
pub struct SudoConfig {
    /// Whether privileged sudo execution is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Explicit list of commands that may be run with sudo (empty = none allowed).
    #[serde(default)]
    pub allowed_commands: Vec<String>,
    /// Whether human confirmation is required before each sudo invocation.
    #[serde(default = "default_sudo_require_confirmation")]
    pub require_confirmation: bool,
    /// Seconds to wait for a confirmation response before timing out.
    #[serde(default = "default_sudo_confirmation_timeout_secs")]
    pub confirmation_timeout_secs: u64,
    /// Filesystem path to the `sudo` binary.
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
#[non_exhaustive]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct KnowledgeConfig {
    /// Whether the knowledge / RAG subsystem is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Vector store backend configuration.
    #[serde(default)]
    pub vector_store: VectorStoreConfig,
    /// Embedding provider configuration.
    #[serde(default)]
    pub embeddings: EmbeddingsConfig,
    /// Document chunking configuration.
    #[serde(default)]
    pub chunking: ChunkingConfig,
}

/// Vector store backend configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct VectorStoreConfig {
    /// Vector store backend identifier (e.g. `"qdrant"`).
    #[serde(default = "default_vector_store_provider")]
    pub provider: String,
    /// gRPC URL for the vector store (e.g. `"http://localhost:6334"`).
    #[serde(default = "default_vector_store_url")]
    pub url: String,
    /// Prefix prepended to all collection names.
    #[serde(default = "default_collection_prefix")]
    pub collection_prefix: String,
    /// Name of the default collection when none is specified.
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

/// Embedding provider configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct EmbeddingsConfig {
    /// Embedding backend identifier: `"local"` (ONNX) or `"openai"`.
    #[serde(default = "default_embedding_provider")]
    pub provider: String,
    /// Model name or path used for embedding generation.
    #[serde(default = "default_embedding_model")]
    pub model: String,
    /// Dimensionality of the embedding vectors produced by this model.
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

/// Document chunking configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct ChunkingConfig {
    /// Maximum number of characters per chunk.
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    /// Number of characters of overlap between consecutive chunks.
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
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct SchedulerConfig {
    /// Whether the cron scheduler is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// How often (in seconds) the scheduler polls for due jobs.
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
    /// Maximum number of jobs that may run concurrently.
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
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct HttpClientConfig {
    /// Whether the HTTP client skill is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Maximum response body size in bytes.
    #[serde(default = "default_http_max_response_bytes")]
    pub max_response_bytes: usize,
    /// Default request timeout in seconds.
    #[serde(default = "default_http_timeout_secs")]
    pub default_timeout_secs: u64,
    /// Domains that are always blocked (e.g. cloud metadata endpoints).
    #[serde(default = "default_http_blocked_domains")]
    pub blocked_domains: Vec<String>,
    /// User-Agent header sent with outbound requests.
    #[serde(default = "default_http_user_agent")]
    pub user_agent: String,
    /// Inbound webhook receiver configuration.
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
    format!("Orka/{}", env!("CARGO_PKG_VERSION"))
}

/// Inbound webhook receiver configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct WebhookConfig {
    /// Whether the inbound webhook receiver is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Public base URL used to construct webhook callback URLs.
    #[serde(default = "default_webhook_base_url")]
    pub base_url: String,
    /// URL path prefix under which webhook endpoints are mounted.
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
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct ExperienceConfig {
    /// Whether the experience / self-learning subsystem is enabled.
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
        if self.os.enabled
            && !matches!(
                self.os.permission_level.to_ascii_lowercase().as_str(),
                "read-only" | "readonly" | "interact" | "write" | "execute" | "admin"
            )
        {
            return Err(crate::Error::Config(format!(
                "os.permission_level must be one of read-only/interact/write/execute/admin, got: '{}'",
                self.os.permission_level
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
}

// ---------------------------------------------------------------------------
// Secret-redacting Debug implementations
// ---------------------------------------------------------------------------

/// Mask the password in a Redis URL so it is safe to log.
///
/// `redis://:password@host` → `redis://:***@host`
/// `redis://user:password@host` → `redis://user:***@host`
fn redact_url(url: &str) -> String {
    if let Some(at_pos) = url.find('@') {
        let scheme_end = url.find("://").map(|i| i + 3).unwrap_or(0);
        let credentials = &url[scheme_end..at_pos];
        if credentials.contains(':') {
            let scheme = &url[..scheme_end];
            let user = credentials.split(':').next().unwrap_or("");
            let rest = &url[at_pos..];
            return if user.is_empty() {
                format!("{scheme}:***{rest}")
            } else {
                format!("{scheme}{user}:***{rest}")
            };
        }
    }
    url.to_string()
}

impl fmt::Debug for RedisConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RedisConfig")
            .field("url", &redact_url(&self.url))
            .finish()
    }
}

impl fmt::Debug for LlmProviderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LlmProviderConfig")
            .field("name", &self.name)
            .field("provider", &self.provider)
            .field("api_key", &self.api_key.as_ref().map(|_| "***"))
            .field(
                "api_key_secret",
                &self.api_key_secret.as_ref().map(|_| "***"),
            )
            .field("api_key_env", &self.api_key_env)
            .field("model", &self.model)
            .field("timeout_secs", &self.timeout_secs)
            .field("max_tokens", &self.max_tokens)
            .field("max_retries", &self.max_retries)
            .field("base_url", &self.base_url)
            .field("prefixes", &self.prefixes)
            .field("cost_per_1k_input_tokens", &self.cost_per_1k_input_tokens)
            .field("cost_per_1k_output_tokens", &self.cost_per_1k_output_tokens)
            .finish()
    }
}

impl fmt::Debug for JwtAuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JwtAuthConfig")
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .field("jwks_uri", &self.jwks_uri)
            .field("secret", &self.secret.as_ref().map(|_| "***"))
            .finish()
    }
}

impl fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthConfig")
            .field("enabled", &self.enabled)
            .field("api_key_header", &self.api_key_header)
            .field("api_keys", &format!("[{} keys]", self.api_keys.len()))
            .field("jwt", &self.jwt)
            .finish()
    }
}

impl fmt::Debug for WebConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebConfig")
            .field("search_provider", &self.search_provider)
            .field("api_key", &self.api_key.as_ref().map(|_| "***"))
            .field("api_key_env", &self.api_key_env)
            .field("searxng_base_url", &self.searxng_base_url)
            .field("max_results", &self.max_results)
            .field("max_read_chars", &self.max_read_chars)
            .field("max_content_chars", &self.max_content_chars)
            .field("cache_ttl_secs", &self.cache_ttl_secs)
            .field("read_timeout_secs", &self.read_timeout_secs)
            .field("user_agent", &self.user_agent)
            .finish()
    }
}

impl fmt::Debug for TelegramAdapterConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TelegramAdapterConfig")
            .field(
                "bot_token_secret",
                &self.bot_token_secret.as_ref().map(|_| "***"),
            )
            .field("workspace", &self.workspace)
            .field("mode", &self.mode)
            .field("webhook_url", &self.webhook_url)
            .field("webhook_port", &self.webhook_port)
            .field("parse_mode", &self.parse_mode)
            .field("streaming", &self.streaming)
            .field("owner_id", &self.owner_id)
            .field("allowed_users", &self.allowed_users)
            .finish()
    }
}

impl fmt::Debug for DiscordAdapterConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiscordAdapterConfig")
            .field(
                "bot_token_secret",
                &self.bot_token_secret.as_ref().map(|_| "***"),
            )
            .field("application_id", &self.application_id)
            .field("workspace", &self.workspace)
            .finish()
    }
}

impl fmt::Debug for SlackAdapterConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlackAdapterConfig")
            .field(
                "bot_token_secret",
                &self.bot_token_secret.as_ref().map(|_| "***"),
            )
            .field("listen_port", &self.listen_port)
            .field("workspace", &self.workspace)
            .finish()
    }
}

impl fmt::Debug for WhatsAppAdapterConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WhatsAppAdapterConfig")
            .field(
                "access_token_secret",
                &self.access_token_secret.as_ref().map(|_| "***"),
            )
            .field("phone_number_id", &self.phone_number_id)
            .field(
                "verify_token_secret",
                &self.verify_token_secret.as_ref().map(|_| "***"),
            )
            .field("listen_port", &self.listen_port)
            .field("workspace", &self.workspace)
            .finish()
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
            experience: ExperienceConfig::default(),
            agents: Vec::new(),
            graph: None,
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
            experience: ExperienceConfig::default(),
            agents: Vec::new(),
            graph: None,
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
