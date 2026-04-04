#![allow(missing_docs)]

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MemoryBackend {
    #[default]
    Auto,
    Redis,
    Memory,
}

const fn default_max_entries() -> usize {
    1000
}

#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct MemoryConfig {
    #[serde(default)]
    pub backend: MemoryBackend,
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: MemoryBackend::default(),
            max_entries: default_max_entries(),
        }
    }
}

impl MemoryConfig {
    pub fn validate(&self) -> orka_core::Result<()> {
        if self.max_entries == 0 {
            return Err(orka_core::Error::Config(
                "memory.max_entries must be greater than 0".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum SecretBackend {
    #[default]
    Redis,
    File,
}

fn default_secret_redis_url() -> String {
    "redis://127.0.0.1:6379".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct SecretRedisConfig {
    #[serde(default = "default_secret_redis_url")]
    pub url: String,
}

impl Default for SecretRedisConfig {
    fn default() -> Self {
        Self {
            url: default_secret_redis_url(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct SecretConfig {
    #[serde(default)]
    pub backend: SecretBackend,
    pub file_path: Option<String>,
    pub encryption_key_path: Option<String>,
    pub encryption_key_env: Option<String>,
    #[serde(flatten)]
    pub redis: SecretRedisConfig,
}

impl SecretConfig {
    pub fn validate(&self) -> orka_core::Result<()> {
        Ok(())
    }
}

const fn default_gateway_rate_limit() -> u32 {
    0
}

const fn default_gateway_dedup_ttl_secs() -> u64 {
    300
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    #[serde(default)]
    pub redis_url: Option<String>,
    #[serde(default = "default_gateway_rate_limit")]
    pub rate_limit: u32,
    #[serde(default = "default_gateway_dedup_ttl_secs")]
    pub dedup_ttl_secs: u64,
    #[serde(default)]
    pub dedup_enabled: bool,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            redis_url: None,
            rate_limit: default_gateway_rate_limit(),
            dedup_ttl_secs: default_gateway_dedup_ttl_secs(),
            dedup_enabled: false,
        }
    }
}

impl GatewayConfig {
    pub fn validate(&self) -> orka_core::Result<()> {
        Ok(())
    }
}

fn default_observe_backend() -> String {
    "stdout".to_string()
}

const fn default_observe_batch_size() -> usize {
    100
}

const fn default_observe_flush_interval_ms() -> u64 {
    1000
}

fn default_audit_output() -> String {
    "stdout".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct ObserveConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_observe_backend")]
    pub backend: String,
    pub otlp_endpoint: Option<String>,
    #[serde(default = "default_observe_batch_size")]
    pub batch_size: usize,
    #[serde(default = "default_observe_flush_interval_ms")]
    pub flush_interval_ms: u64,
    #[serde(default)]
    pub service_name: String,
    #[serde(default)]
    pub service_version: String,
}

impl Default for ObserveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: default_observe_backend(),
            otlp_endpoint: None,
            batch_size: default_observe_batch_size(),
            flush_interval_ms: default_observe_flush_interval_ms(),
            service_name: "orka".to_string(),
            service_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct AuditConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_audit_output")]
    pub output: String,
    pub path: Option<PathBuf>,
    pub redis_key: Option<String>,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            output: default_audit_output(),
            path: None,
            redis_key: None,
        }
    }
}

impl ObserveConfig {
    pub fn validate(&self) -> orka_core::Result<()> {
        if self.batch_size == 0 {
            return Err(orka_core::Error::Config(
                "observe.batch_size must be greater than 0".into(),
            ));
        }
        if self.flush_interval_ms == 0 {
            return Err(orka_core::Error::Config(
                "observe.flush_interval_ms must be greater than 0".into(),
            ));
        }
        if self.backend == "otlp" && self.otlp_endpoint.as_deref().is_none_or(str::is_empty) {
            return Err(orka_core::Error::Config(
                "observe.otlp_endpoint must be set when backend is 'otlp'".into(),
            ));
        }
        Ok(())
    }
}

impl AuditConfig {
    pub fn validate(&self) -> orka_core::Result<()> {
        if self.enabled {
            match self.output.as_str() {
                "file" => {
                    if self.path.is_none() {
                        return Err(orka_core::Error::Config(
                            "audit.path must be set when output is 'file'".into(),
                        ));
                    }
                }
                "redis" => {
                    if self.redis_key.as_deref().is_none_or(str::is_empty) {
                        return Err(orka_core::Error::Config(
                            "audit.redis_key must be set when output is 'redis'".into(),
                        ));
                    }
                }
                "stdout" => {}
                other => {
                    return Err(orka_core::Error::Config(format!(
                        "audit.output: unknown value '{other}' (expected 'stdout', 'file', or 'redis')"
                    )));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchProviderKind {
    Tavily,
    Brave,
    Searxng,
    None,
}

fn default_search_provider() -> SearchProviderKind {
    SearchProviderKind::None
}

fn default_max_results() -> usize {
    5
}

fn default_max_read_chars() -> usize {
    20_000
}

fn default_max_content_chars() -> usize {
    8_000
}

fn default_cache_ttl_secs() -> u64 {
    3600
}

fn default_read_timeout_secs() -> u64 {
    15
}

fn default_web_user_agent() -> String {
    format!("Orka/{} (Web Agent)", env!("CARGO_PKG_VERSION"))
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebConfig {
    #[serde(default = "default_search_provider")]
    pub search_provider: SearchProviderKind,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub searxng_base_url: Option<String>,
    #[serde(default = "default_max_results")]
    pub max_results: usize,
    #[serde(default = "default_max_read_chars")]
    pub max_read_chars: usize,
    #[serde(default = "default_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
    #[serde(default = "default_max_content_chars")]
    pub max_content_chars: usize,
    #[serde(default = "default_read_timeout_secs")]
    pub read_timeout_secs: u64,
    #[serde(default = "default_web_user_agent")]
    pub user_agent: String,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            search_provider: default_search_provider(),
            api_key: None,
            api_key_env: None,
            searxng_base_url: None,
            max_results: default_max_results(),
            max_read_chars: default_max_read_chars(),
            cache_ttl_secs: default_cache_ttl_secs(),
            max_content_chars: default_max_content_chars(),
            read_timeout_secs: default_read_timeout_secs(),
            user_agent: default_web_user_agent(),
        }
    }
}

impl WebConfig {
    pub fn validate(&self) -> orka_core::Result<()> {
        if self.search_provider == SearchProviderKind::Searxng && self.searxng_base_url.is_none() {
            return Err(orka_core::Error::Config(
                "web.searxng_base_url is required when web.search_provider = \"searxng\"".into(),
            ));
        }
        if self.max_results == 0 {
            return Err(orka_core::Error::Config(
                "web.max_results must be greater than 0".into(),
            ));
        }
        Ok(())
    }
}

const fn default_http_timeout_secs() -> u64 {
    30
}

const fn default_max_redirects() -> usize {
    10
}

fn default_webhook_method() -> String {
    "POST".to_string()
}

const fn default_webhook_max_retries() -> u32 {
    3
}

const fn default_webhook_retry_delay_secs() -> u64 {
    5
}

#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct HttpClientConfig {
    #[serde(default = "default_http_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_redirects")]
    pub max_redirects: usize,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub default_headers: Vec<(String, String)>,
    #[serde(default)]
    pub webhooks: Vec<WebhookConfig>,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            timeout_secs: default_http_timeout_secs(),
            max_redirects: default_max_redirects(),
            user_agent: None,
            default_headers: Vec::new(),
            webhooks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct WebhookConfig {
    pub name: String,
    pub url: String,
    #[serde(default = "default_webhook_method")]
    pub method: String,
    pub secret: Option<String>,
    #[serde(default)]
    pub retry: WebhookRetryConfig,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            url: String::new(),
            method: default_webhook_method(),
            secret: None,
            retry: WebhookRetryConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct WebhookRetryConfig {
    #[serde(default = "default_webhook_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_webhook_retry_delay_secs")]
    pub delay_secs: u64,
}

impl Default for WebhookRetryConfig {
    fn default() -> Self {
        Self {
            max_retries: default_webhook_max_retries(),
            delay_secs: default_webhook_retry_delay_secs(),
        }
    }
}

impl HttpClientConfig {
    pub fn validate(&self) -> orka_core::Result<()> {
        if self.timeout_secs == 0 {
            return Err(orka_core::Error::Config(
                "http.timeout_secs must be greater than 0".into(),
            ));
        }
        Ok(())
    }
}

fn default_sandbox_backend() -> String {
    "process".to_string()
}

const fn default_timeout_secs() -> u64 {
    30
}

const fn default_max_memory_bytes() -> usize {
    64 * 1024 * 1024
}

const fn default_max_output_bytes() -> usize {
    1024 * 1024
}

const fn default_max_pids() -> usize {
    10
}

#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct SandboxConfig {
    #[serde(default = "default_sandbox_backend")]
    pub backend: String,
    #[serde(default)]
    pub limits: SandboxLimitsConfig,
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub denied_paths: Vec<String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            backend: default_sandbox_backend(),
            limits: SandboxLimitsConfig::default(),
            allowed_paths: Vec::new(),
            denied_paths: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct SandboxLimitsConfig {
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_memory_bytes")]
    pub max_memory_bytes: usize,
    #[serde(default = "default_max_output_bytes")]
    pub max_output_bytes: usize,
    #[serde(default)]
    pub max_open_files: Option<usize>,
    #[serde(default = "default_max_pids")]
    pub max_pids: usize,
}

impl Default for SandboxLimitsConfig {
    fn default() -> Self {
        Self {
            timeout_secs: default_timeout_secs(),
            max_memory_bytes: default_max_memory_bytes(),
            max_output_bytes: default_max_output_bytes(),
            max_open_files: None,
            max_pids: default_max_pids(),
        }
    }
}

impl SandboxConfig {
    pub fn validate(&self) -> orka_core::Result<()> {
        if self.limits.timeout_secs == 0 {
            return Err(orka_core::Error::Config(
                "sandbox.limits.timeout_secs must be greater than 0".into(),
            ));
        }
        if self.limits.max_memory_bytes == 0 {
            return Err(orka_core::Error::Config(
                "sandbox.limits.max_memory_bytes must be greater than 0".into(),
            ));
        }
        if self.limits.max_pids == 0 {
            return Err(orka_core::Error::Config(
                "sandbox.limits.max_pids must be greater than 0".into(),
            ));
        }
        Ok(())
    }
}
