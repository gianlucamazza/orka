use serde::Deserialize;

/// Bus backend options.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BusBackend {
    /// Redis Streams backend.
    #[default]
    Redis,
    /// NATS backend (future).
    Nats,
    /// In-memory backend (testing only).
    Memory,
}

const fn default_bus_block_ms() -> u64 {
    5000
}

const fn default_bus_batch_size() -> usize {
    100
}

const fn default_bus_backoff_initial_secs() -> u64 {
    1
}

const fn default_bus_backoff_max_secs() -> u64 {
    60
}

/// Message bus configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct BusConfig {
    /// Bus backend to use.
    #[serde(default)]
    pub backend: BusBackend,
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

impl Default for BusConfig {
    fn default() -> Self {
        Self {
            backend: BusBackend::default(),
            block_ms: default_bus_block_ms(),
            batch_size: default_bus_batch_size(),
            backoff_initial_secs: default_bus_backoff_initial_secs(),
            backoff_max_secs: default_bus_backoff_max_secs(),
        }
    }
}

impl BusConfig {
    /// Validate bus configuration.
    pub fn validate(&self) -> orka_core::Result<()> {
        if self.batch_size == 0 {
            return Err(orka_core::Error::Config(
                "bus.batch_size must be greater than 0".into(),
            ));
        }
        if self.backoff_initial_secs > self.backoff_max_secs {
            return Err(orka_core::Error::Config(
                "bus.backoff_initial_secs must be <= bus.backoff_max_secs".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_redis_backend() {
        let config = BusConfig::default();
        assert!(matches!(config.backend, BusBackend::Redis));
        assert_eq!(config.block_ms, 5000);
        assert_eq!(config.batch_size, 100);
    }
}
