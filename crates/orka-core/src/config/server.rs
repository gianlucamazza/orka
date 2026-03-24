//! HTTP server configuration.

use serde::Deserialize;

use crate::config::defaults;

/// HTTP server bind configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct ServerConfig {
    /// IP address or hostname to bind on.
    #[serde(default = "defaults::default_host")]
    pub host: String,
    /// TCP port to listen on.
    #[serde(default = "defaults::default_port")]
    pub port: u16,
}

impl ServerConfig {
    /// Create a new ServerConfig with default values.
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
    ///
    /// # Errors
    ///
    /// Returns an error if the port is 0.
    pub fn validate(&self) -> crate::Result<()> {
        if self.port == 0 {
            return Err(crate::Error::Config(
                "server.port must be in range 1-65535".into(),
            ));
        }
        if self.host.is_empty() {
            return Err(crate::Error::Config("server.host must not be empty".into()));
        }
        Ok(())
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: defaults::default_host().to_string(),
            port: defaults::default_port(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_server_config() {
        let config = ServerConfig::default();
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8080);
    }

    #[test]
    fn bind_address_formatting() {
        let config = ServerConfig {
            host: "0.0.0.0".to_string(),
            port: 3000,
        };
        assert_eq!(config.bind_address(), "0.0.0.0:3000");
    }

    #[test]
    fn validation_rejects_port_zero() {
        let config = ServerConfig {
            host: "localhost".to_string(),
            port: 0,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validation_rejects_empty_host() {
        let config = ServerConfig {
            host: "".to_string(),
            port: 8080,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validation_accepts_valid_config() {
        let config = ServerConfig {
            host: "0.0.0.0".to_string(),
            port: 443,
        };
        assert!(config.validate().is_ok());
    }
}
