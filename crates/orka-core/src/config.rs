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

impl OrkaConfig {
    /// Load configuration from file + environment variables.
    ///
    /// Looks for `orka.toml` in the given path (or current directory),
    /// then overlays `ORKA_` prefixed environment variables.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let mut builder = Config::builder();

        let config_path = path
            .map(|p| p.to_path_buf())
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
        });
        assert_eq!(cfg.server.port, 8080);
        assert_eq!(cfg.bus.backend, "redis");
    }
}
