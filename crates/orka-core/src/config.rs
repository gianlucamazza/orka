use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use std::path::Path;

/// Top-level Orka configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct OrkaConfig {
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
    pub a2a: A2aConfig,
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
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DiscordAdapterConfig {
    pub bot_token_secret: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackAdapterConfig {
    pub bot_token_secret: Option<String>,
    #[serde(default = "default_slack_port")]
    pub listen_port: u16,
}

impl Default for SlackAdapterConfig {
    fn default() -> Self {
        Self {
            bot_token_secret: None,
            listen_port: default_slack_port(),
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
}

impl Default for WhatsAppAdapterConfig {
    fn default() -> Self {
        Self {
            access_token_secret: None,
            phone_number_id: None,
            verify_token_secret: None,
            listen_port: default_whatsapp_port(),
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
}

impl Default for CustomAdapterConfig {
    fn default() -> Self {
        Self {
            host: default_custom_host(),
            port: default_custom_port(),
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
    /// DEPRECATED: use [[llm.providers]] instead.
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default = "default_llm_model")]
    pub model: String,
    /// DEPRECATED: use [[llm.providers]] instead.
    #[serde(default)]
    pub api_key_secret: Option<String>,
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
    /// Convert legacy flat fields (`provider`, `api_key_secret`, …) into a
    /// single entry in `providers`. No-op when `providers` is already populated.
    pub fn normalize(&mut self) {
        if !self.providers.is_empty() {
            return;
        }
        if let Some(provider) = self.provider.take() {
            self.providers.push(LlmProviderConfig {
                name: provider.clone(),
                provider: provider.clone(),
                api_key_secret: self.api_key_secret.take(),
                api_key: None,
                api_key_env: None,
                model: self.model.clone(),
                timeout_secs: self.timeout_secs,
                max_tokens: self.max_tokens,
                max_retries: self.max_retries,
                base_url: None,
                prefixes: Vec::new(),
            });
        }
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: None,
            model: default_llm_model(),
            api_key_secret: None,
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
    #[serde(default = "default_llm_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_llm_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_llm_max_retries")]
    pub max_retries: u32,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub prefixes: Vec<String>,
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
        }
    }
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

#[derive(Debug, Clone, Deserialize)]
pub struct A2aConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub url: Option<String>,
}

impl Default for A2aConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: None,
        }
    }
}

impl OrkaConfig {
    /// Validate the loaded configuration.
    ///
    /// Also normalizes the LLM config so that legacy flat fields are converted
    /// into the canonical `providers` vec.
    pub fn validate(&mut self) -> crate::Result<()> {
        self.llm.normalize();

        if self.server.port == 0 {
            return Err(crate::Error::Config(
                "server.port must be in range 1-65535".into(),
            ));
        }

        if let Some(ref custom) = self.adapters.custom {
            if custom.port == 0 {
                return Err(crate::Error::Config(
                    "adapters.custom.port must be in range 1-65535".into(),
                ));
            }
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

        Ok(())
    }

    /// Load configuration from file + environment variables.
    ///
    /// Resolution order for the config file path:
    /// 1. Explicit `path` argument
    /// 2. `ORKA_CONFIG` environment variable
    /// 3. `orka.toml` in the current directory
    ///
    /// Then overlays `ORKA__` prefixed environment variables.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let mut builder = Config::builder();

        let config_path = path
            .map(|p| p.to_path_buf())
            .or_else(|| {
                std::env::var("ORKA_CONFIG")
                    .ok()
                    .map(std::path::PathBuf::from)
            })
            .unwrap_or_else(|| "orka.toml".into());

        if config_path.exists() {
            builder = builder.add_source(File::from(config_path));
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
            server: ServerConfig::default(),
            bus: BusConfig::default(),
            redis: RedisConfig::default(),
            logging: LoggingConfig::default(),
            workspace_dir: default_workspace_dir(),
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
            observe: ObserveConfig::default(),
            gateway: GatewayConfig::default(),
            mcp: McpConfig::default(),
            guardrails: GuardrailsConfig::default(),
            web: WebConfig::default(),
            a2a: A2aConfig::default(),
        });
        assert_eq!(cfg.server.port, 8080);
        assert_eq!(cfg.bus.backend, "redis");
        assert!(!cfg.auth.enabled);
        assert_eq!(cfg.sandbox.backend, "process");
    }

    fn valid_config() -> OrkaConfig {
        OrkaConfig {
            server: ServerConfig::default(),
            bus: BusConfig::default(),
            redis: RedisConfig::default(),
            logging: LoggingConfig::default(),
            workspace_dir: ".".into(), // current dir exists
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
            observe: ObserveConfig::default(),
            gateway: GatewayConfig::default(),
            mcp: McpConfig::default(),
            guardrails: GuardrailsConfig::default(),
            web: WebConfig::default(),
            a2a: A2aConfig::default(),
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
    fn llm_provider_without_key_rejected() {
        let mut cfg = valid_config();
        cfg.llm.provider = Some("anthropic".into());
        cfg.llm.api_key_secret = None;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn normalize_legacy_to_providers() {
        let mut llm = LlmConfig {
            provider: Some("anthropic".into()),
            api_key_secret: Some("my_key".into()),
            model: "claude-sonnet-4-6".into(),
            ..LlmConfig::default()
        };
        llm.normalize();
        assert_eq!(llm.providers.len(), 1);
        assert_eq!(llm.providers[0].name, "anthropic");
        assert_eq!(llm.providers[0].provider, "anthropic");
        assert_eq!(llm.providers[0].api_key_secret.as_deref(), Some("my_key"));
        assert_eq!(llm.providers[0].model, "claude-sonnet-4-6");
        // Legacy fields cleared
        assert!(llm.provider.is_none());
        assert!(llm.api_key_secret.is_none());
    }

    #[test]
    fn normalize_noop_when_providers_set() {
        let mut llm = LlmConfig {
            provider: Some("should_be_ignored".into()),
            providers: vec![LlmProviderConfig {
                name: "existing".into(),
                provider: "openai".into(),
                api_key_secret: Some("k".into()),
                api_key: None,
                api_key_env: None,
                model: "gpt-4".into(),
                timeout_secs: 30,
                max_tokens: 4096,
                max_retries: 2,
                base_url: None,
                prefixes: Vec::new(),
            }],
            ..LlmConfig::default()
        };
        llm.normalize();
        assert_eq!(llm.providers.len(), 1);
        assert_eq!(llm.providers[0].name, "existing");
        // Legacy field untouched (not consumed)
        assert_eq!(llm.provider.as_deref(), Some("should_be_ignored"));
    }

    #[test]
    fn normalize_empty() {
        let mut llm = LlmConfig::default();
        llm.normalize();
        assert!(llm.providers.is_empty());
    }
}
