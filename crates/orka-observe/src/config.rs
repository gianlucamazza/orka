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

impl ObserveConfig {
    /// Validate the observability configuration.
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
    /// Validate the audit configuration.
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

#[cfg(test)]
mod tests {
    use super::*;

    // --- ObserveConfig::validate ---

    #[test]
    fn observe_default_config_is_valid() {
        assert!(ObserveConfig::default().validate().is_ok());
    }

    #[test]
    fn observe_batch_size_zero_is_invalid() {
        let cfg = ObserveConfig {
            batch_size: 0,
            ..ObserveConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn observe_flush_interval_zero_is_invalid() {
        let cfg = ObserveConfig {
            flush_interval_ms: 0,
            ..ObserveConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn observe_otlp_backend_without_endpoint_is_invalid() {
        let cfg = ObserveConfig {
            backend: "otlp".to_string(),
            otlp_endpoint: None,
            ..ObserveConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn observe_otlp_backend_with_endpoint_is_valid() {
        let cfg = ObserveConfig {
            backend: "otlp".to_string(),
            otlp_endpoint: Some("http://localhost:4317".to_string()),
            ..ObserveConfig::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn observe_otlp_backend_with_empty_endpoint_is_invalid() {
        let cfg = ObserveConfig {
            backend: "otlp".to_string(),
            otlp_endpoint: Some(String::new()),
            ..ObserveConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    // --- AuditConfig::validate ---

    #[test]
    fn audit_disabled_is_always_valid() {
        for output in &["file", "redis", "stdout", "unknown"] {
            let cfg = AuditConfig {
                enabled: false,
                output: (*output).to_string(),
                path: None,
                redis_key: None,
            };
            assert!(
                cfg.validate().is_ok(),
                "should be valid when disabled: {output}"
            );
        }
    }

    #[test]
    fn audit_enabled_stdout_is_valid() {
        let cfg = AuditConfig {
            enabled: true,
            output: "stdout".to_string(),
            ..AuditConfig::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn audit_enabled_file_without_path_is_invalid() {
        let cfg = AuditConfig {
            enabled: true,
            output: "file".to_string(),
            path: None,
            redis_key: None,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn audit_enabled_file_with_path_is_valid() {
        let cfg = AuditConfig {
            enabled: true,
            output: "file".to_string(),
            path: Some("/var/log/audit.log".into()),
            redis_key: None,
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn audit_enabled_redis_without_key_is_invalid() {
        let cfg = AuditConfig {
            enabled: true,
            output: "redis".to_string(),
            path: None,
            redis_key: None,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn audit_enabled_redis_with_empty_key_is_invalid() {
        let cfg = AuditConfig {
            enabled: true,
            output: "redis".to_string(),
            path: None,
            redis_key: Some(String::new()),
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn audit_enabled_redis_with_key_is_valid() {
        let cfg = AuditConfig {
            enabled: true,
            output: "redis".to_string(),
            path: None,
            redis_key: Some("orka:audit".to_string()),
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn audit_enabled_unknown_output_is_invalid() {
        let cfg = AuditConfig {
            enabled: true,
            output: "kafka".to_string(),
            path: None,
            redis_key: None,
        };
        assert!(cfg.validate().is_err());
    }
}
