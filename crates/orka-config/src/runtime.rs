use serde::Deserialize;

/// System-wide configuration file path (native package installs).
pub const SYSTEM_CONFIG_PATH: &str = "/etc/orka/orka.toml";

/// A named workspace entry for multi-workspace support.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceEntry {
    /// Unique name for this workspace, used for routing and CLI selection.
    pub name: String,
    /// Filesystem path to the workspace directory.
    pub dir: String,
}

/// Default config schema version used for newly composed configs.
pub(crate) const fn default_config_version() -> u32 {
    6
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

fn default_host() -> String {
    "127.0.0.1".to_string()
}

const fn default_port() -> u16 {
    8080
}

fn default_redis_url() -> String {
    "redis://127.0.0.1:6379".to_string()
}

const fn default_concurrency() -> usize {
    4
}

const fn default_retry_base_delay_ms() -> u64 {
    1000
}

const fn default_max_retries() -> u32 {
    3
}

/// HTTP server bind configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct ServerConfig {
    /// IP address or hostname to bind on.
    #[serde(default = "default_host")]
    pub host: String,
    /// TCP port to listen on.
    #[serde(default = "default_port")]
    pub port: u16,
}

impl ServerConfig {
    /// Create a new `ServerConfig` with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the bind address as a string (e.g., "127.0.0.1:8080").
    #[must_use]
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Validate the server configuration.
    pub fn validate(&self) -> orka_core::Result<()> {
        if self.port == 0 {
            return Err(orka_core::Error::Config(
                "server.port must be in range 1-65535".into(),
            ));
        }
        if self.host.is_empty() {
            return Err(orka_core::Error::Config(
                "server.host must not be empty".into(),
            ));
        }
        Ok(())
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

/// Redis connection configuration.
#[derive(Clone, Deserialize)]
#[non_exhaustive]
pub struct RedisConfig {
    /// Redis connection URL (e.g. `"redis://127.0.0.1:6379"`).
    #[serde(default = "default_redis_url")]
    pub url: String,
}

impl RedisConfig {
    /// Create a new `RedisConfig` with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate the Redis configuration.
    pub fn validate(&self) -> orka_core::Result<()> {
        if !self.url.starts_with("redis://") && !self.url.starts_with("rediss://") {
            return Err(orka_core::Error::Config(format!(
                "redis.url must start with redis:// or rediss://, got: {}",
                self.url
            )));
        }
        Ok(())
    }
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: default_redis_url(),
        }
    }
}

impl std::fmt::Debug for RedisConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisConfig")
            .field("url", &redact_url(&self.url))
            .finish()
    }
}

fn redact_url(url: &str) -> String {
    if let Some(at_pos) = url.find('@') {
        let scheme_end = url.find("://").map_or(0, |i| i + 3);
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

/// Structured logging configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct LoggingConfig {
    /// Log level filter.
    #[serde(default)]
    pub level: LogLevel,
    /// Emit logs as JSON (useful for log aggregators).
    #[serde(default)]
    pub json: bool,
}

/// Worker pool configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct WorkerConfig {
    /// Number of concurrent workers.
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    /// Base delay between retries in milliseconds.
    #[serde(default = "default_retry_base_delay_ms")]
    pub retry_base_delay_ms: u64,
}

impl WorkerConfig {
    /// Validate the worker configuration.
    pub fn validate(&self) -> orka_core::Result<()> {
        if self.concurrency == 0 {
            return Err(orka_core::Error::Config(
                "worker.concurrency must be greater than 0".into(),
            ));
        }
        Ok(())
    }
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            concurrency: default_concurrency(),
            retry_base_delay_ms: default_retry_base_delay_ms(),
        }
    }
}

/// Queue retry policy configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct QueueConfig {
    /// Maximum number of retries for failed messages.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redis_default_url() {
        assert_eq!(RedisConfig::default().url, "redis://127.0.0.1:6379");
    }

    #[test]
    fn redis_validates_url() {
        assert!(
            RedisConfig {
                url: "redis://localhost:6379".to_string(),
            }
            .validate()
            .is_ok()
        );
        assert!(
            RedisConfig {
                url: "rediss://localhost:6380".to_string(),
            }
            .validate()
            .is_ok()
        );
        assert!(
            RedisConfig {
                url: "http://localhost:6379".to_string(),
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn server_default_values() {
        let config = ServerConfig::default();
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8080);
    }

    #[test]
    fn server_bind_address_and_validation() {
        let config = ServerConfig {
            host: "0.0.0.0".to_string(),
            port: 3000,
        };
        assert_eq!(config.bind_address(), "0.0.0.0:3000");
        assert!(config.validate().is_ok());
    }

    #[test]
    fn logging_defaults_to_info_text_output() {
        let config = LoggingConfig::default();

        assert_eq!(config.level, LogLevel::Info);
        assert!(!config.json);
    }

    #[test]
    fn config_constants_match_expected_values() {
        assert_eq!(SYSTEM_CONFIG_PATH, "/etc/orka/orka.toml");
        assert_eq!(default_config_version(), 6);
    }

    #[test]
    fn worker_validation_rejects_zero_concurrency() {
        assert!(
            WorkerConfig {
                concurrency: 0,
                retry_base_delay_ms: 1000,
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn queue_defaults_to_three_retries() {
        assert_eq!(QueueConfig::default().max_retries, 3);
    }
}
