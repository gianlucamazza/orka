//! Infrastructure configuration (Redis, Bus, Queue, Session, Memory).

use serde::Deserialize;

use crate::config::{
    defaults,
    primitives::{BusBackend, MemoryBackend},
};

/// Redis connection configuration.
#[derive(Clone, Deserialize)]
#[non_exhaustive]
pub struct RedisConfig {
    /// Redis connection URL (e.g. `"redis://127.0.0.1:6379"`).
    #[serde(default = "defaults::default_redis_url")]
    pub url: String,
}

impl RedisConfig {
    /// Create a new RedisConfig with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate the Redis configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the URL doesn't start with `redis://` or
    /// `rediss://`.
    pub fn validate(&self) -> crate::Result<()> {
        if !self.url.starts_with("redis://") && !self.url.starts_with("rediss://") {
            return Err(crate::Error::Config(format!(
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
            url: defaults::default_redis_url().to_string(),
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

/// Mask the password in a Redis URL so it is safe to log.
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

/// Message bus configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct BusConfig {
    /// Bus backend to use.
    #[serde(default)]
    pub backend: BusBackend,
    /// XREADGROUP BLOCK timeout in milliseconds.
    #[serde(default = "defaults::default_bus_block_ms")]
    pub block_ms: u64,
    /// XREADGROUP COUNT per read.
    #[serde(default = "defaults::default_bus_batch_size")]
    pub batch_size: usize,
    /// Initial backoff on connection error (seconds).
    #[serde(default = "defaults::default_bus_backoff_initial_secs")]
    pub backoff_initial_secs: u64,
    /// Maximum backoff cap (seconds).
    #[serde(default = "defaults::default_bus_backoff_max_secs")]
    pub backoff_max_secs: u64,
}

impl Default for BusConfig {
    fn default() -> Self {
        Self {
            backend: BusBackend::default(),
            block_ms: defaults::default_bus_block_ms(),
            batch_size: defaults::default_bus_batch_size(),
            backoff_initial_secs: defaults::default_bus_backoff_initial_secs(),
            backoff_max_secs: defaults::default_bus_backoff_max_secs(),
        }
    }
}

/// Priority queue configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct QueueConfig {
    /// Maximum number of retries for failed messages.
    #[serde(default = "defaults::default_max_retries")]
    pub max_retries: u32,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_retries: defaults::default_max_retries(),
        }
    }
}

/// Session store configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct SessionConfig {
    /// Session time-to-live in seconds.
    #[serde(default = "defaults::default_session_ttl_secs")]
    pub ttl_secs: u64,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            ttl_secs: defaults::default_session_ttl_secs(),
        }
    }
}

/// In-memory (Redis) memory store configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct MemoryConfig {
    /// Memory backend to use.
    #[serde(default)]
    pub backend: MemoryBackend,
    /// Maximum number of entries to keep in memory.
    #[serde(default = "defaults::default_max_entries")]
    pub max_entries: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: MemoryBackend::default(),
            max_entries: defaults::default_max_entries(),
        }
    }
}

/// Worker pool configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct WorkerConfig {
    /// Number of concurrent workers.
    #[serde(default = "defaults::default_concurrency")]
    pub concurrency: usize,
    /// Base delay between retries in milliseconds.
    #[serde(default = "defaults::default_retry_base_delay_ms")]
    pub retry_base_delay_ms: u64,
}

impl WorkerConfig {
    /// Validate the worker configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if concurrency is 0.
    pub fn validate(&self) -> crate::Result<()> {
        if self.concurrency == 0 {
            return Err(crate::Error::Config(
                "worker.concurrency must be greater than 0".into(),
            ));
        }
        Ok(())
    }
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            concurrency: defaults::default_concurrency(),
            retry_base_delay_ms: defaults::default_retry_base_delay_ms(),
        }
    }
}

/// Structured logging configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct LoggingConfig {
    /// Log level filter.
    #[serde(default = "defaults::default_log_level")]
    pub level: String,
    /// Emit logs as JSON (useful for log aggregators).
    #[serde(default)]
    pub json: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: defaults::default_log_level().to_string(),
            json: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redis_default_url() {
        let config = RedisConfig::default();
        assert_eq!(config.url, "redis://127.0.0.1:6379");
    }

    #[test]
    fn redis_validates_url() {
        let valid = RedisConfig {
            url: "redis://localhost:6379".to_string(),
        };
        assert!(valid.validate().is_ok());

        let valid_tls = RedisConfig {
            url: "rediss://localhost:6380".to_string(),
        };
        assert!(valid_tls.validate().is_ok());

        let invalid = RedisConfig {
            url: "http://localhost:6379".to_string(),
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn worker_validates_concurrency() {
        let invalid = WorkerConfig {
            concurrency: 0,
            retry_base_delay_ms: 1000,
        };
        assert!(invalid.validate().is_err());

        let valid = WorkerConfig {
            concurrency: 4,
            retry_base_delay_ms: 1000,
        };
        assert!(valid.validate().is_ok());
    }

    #[test]
    fn bus_default_config() {
        let config = BusConfig::default();
        assert!(matches!(config.backend, BusBackend::Redis));
        assert_eq!(config.block_ms, 5000);
        assert_eq!(config.batch_size, 100);
    }
}
