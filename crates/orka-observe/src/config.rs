use std::path::PathBuf;

use serde::Deserialize;

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

/// Observability (metrics/tracing) configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct ObserveConfig {
    /// Enable observability.
    #[serde(default)]
    pub enabled: bool,
    /// Backend: "stdout", "prometheus", "otlp".
    #[serde(default = "default_observe_backend")]
    pub backend: String,
    /// OTLP endpoint URL.
    pub otlp_endpoint: Option<String>,
    /// Metrics batch size.
    #[serde(default = "default_observe_batch_size")]
    pub batch_size: usize,
    /// Flush interval in milliseconds.
    #[serde(default = "default_observe_flush_interval_ms")]
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
            backend: default_observe_backend(),
            otlp_endpoint: None,
            batch_size: default_observe_batch_size(),
            flush_interval_ms: default_observe_flush_interval_ms(),
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
    #[serde(default = "default_audit_output")]
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
            output: default_audit_output(),
            path: None,
            redis_key: None,
        }
    }
}
