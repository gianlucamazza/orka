//! Observability, audit, and gateway configuration.

use std::path::PathBuf;

use serde::Deserialize;

use crate::config::defaults;

/// Observability (metrics/tracing) configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct ObserveConfig {
    /// Enable observability.
    #[serde(default)]
    pub enabled: bool,
    /// Backend: "stdout", "prometheus", "otlp".
    #[serde(default = "defaults::default_observe_backend")]
    pub backend: String,
    /// OTLP endpoint URL.
    pub otlp_endpoint: Option<String>,
    /// Metrics batch size.
    #[serde(default = "defaults::default_observe_batch_size")]
    pub batch_size: usize,
    /// Flush interval in milliseconds.
    #[serde(default = "defaults::default_observe_flush_interval_ms")]
    pub flush_interval_ms: u64,
    /// Service name for telemetry.
    #[serde(default)]
    pub service_name: String,
    /// Service version.
    #[serde(default)]
    pub service_version: String,
}

impl Default for ObserveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: defaults::default_observe_backend(),
            otlp_endpoint: None,
            batch_size: defaults::default_observe_batch_size(),
            flush_interval_ms: defaults::default_observe_flush_interval_ms(),
            service_name: "orka".to_string(),
            service_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

/// Skill invocation audit log configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct AuditConfig {
    /// Enable audit logging.
    #[serde(default)]
    pub enabled: bool,
    /// Output destination: "stdout", "file", "redis".
    #[serde(default = "defaults::default_audit_output")]
    pub output: String,
    /// File path (if output = "file").
    pub path: Option<PathBuf>,
    /// Redis stream key (if output = "redis").
    pub redis_key: Option<String>,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            output: defaults::default_audit_output(),
            path: None,
            redis_key: None,
        }
    }
}

/// API gateway rate limiting and deduplication configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct GatewayConfig {
    /// Maximum requests per minute per session (0 = unlimited).
    #[serde(default = "defaults::default_gateway_rate_limit")]
    pub rate_limit: u32,
    /// Deduplication TTL in seconds.
    #[serde(default = "defaults::default_gateway_dedup_ttl_secs")]
    pub dedup_ttl_secs: u64,
    /// Enable request deduplication.
    #[serde(default)]
    pub dedup_enabled: bool,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            rate_limit: defaults::default_gateway_rate_limit(),
            dedup_ttl_secs: defaults::default_gateway_dedup_ttl_secs(),
            dedup_enabled: false,
        }
    }
}

impl GatewayConfig {
    /// Validate the gateway configuration.
    pub fn validate(&self) -> crate::Result<()> {
        Ok(())
    }
}
