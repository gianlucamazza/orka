//! Publish/subscribe message bus backed by Redis Streams.
//!
//! Provides the [`RedisBus`] implementation of
//! [`orka_core::traits::MessageBus`] and a [`create_bus`] factory that selects
//! the backend from configuration.

/// Message bus configuration.
pub mod config;
/// Redis Streams implementation of the message bus.
pub mod redis_bus;

use std::sync::Arc;

use orka_core::traits::MessageBus;

pub use self::{
    config::{BusBackend, BusConfig},
    redis_bus::RedisBus,
};

/// Create a [`MessageBus`] from the given bus and Redis configuration.
pub fn create_bus(config: &BusConfig, redis_url: &str) -> orka_core::Result<Arc<dyn MessageBus>> {
    match config.backend {
        BusBackend::Redis => {
            let bus = RedisBus::new(redis_url, config)?;
            Ok(Arc::new(bus))
        }
        BusBackend::Memory => Ok(Arc::new(orka_core::testing::InMemoryBus::new())),
        BusBackend::Nats => Err(orka_core::Error::bus(
            "NATS bus backend not yet implemented",
        )),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn config_with_backend(backend: &str) -> BusConfig {
        serde_json::from_value(serde_json::json!({ "backend": backend })).unwrap()
    }

    #[test]
    fn memory_backend_succeeds() {
        let config = config_with_backend("memory");
        let bus = create_bus(&config, "redis://127.0.0.1:6379");
        assert!(bus.is_ok());
    }

    #[test]
    fn unsupported_backend_errors() {
        let config = config_with_backend("nats");
        let bus = create_bus(&config, "redis://127.0.0.1:6379");
        assert!(bus.is_err());
    }
}
