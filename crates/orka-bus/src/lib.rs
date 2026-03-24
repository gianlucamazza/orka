//! Publish/subscribe message bus backed by Redis Streams.
//!
//! Provides the [`RedisBus`] implementation of
//! [`orka_core::traits::MessageBus`] and a [`create_bus`] factory that selects
//! the backend from configuration.

#![warn(missing_docs)]

/// Redis Streams implementation of the message bus.
pub mod redis_bus;

use std::sync::Arc;

use orka_core::{config::primitives::BusBackend, traits::MessageBus};

pub use crate::redis_bus::RedisBus;

/// Create a [`MessageBus`] from the given configuration.
pub fn create_bus(
    config: &orka_core::config::OrkaConfig,
) -> orka_core::Result<Arc<dyn MessageBus>> {
    match config.bus.backend {
        BusBackend::Redis => {
            let bus = RedisBus::new(&config.redis.url, &config.bus)?;
            Ok(Arc::new(bus))
        }
        BusBackend::Memory => Ok(Arc::new(orka_core::testing::InMemoryBus::new())),
        BusBackend::Nats => Err(orka_core::Error::bus(
            "NATS bus backend not yet implemented",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_backend(backend: &str) -> orka_core::config::OrkaConfig {
        serde_json::from_value(serde_json::json!({
            "bus": { "backend": backend }
        }))
        .unwrap()
    }

    #[test]
    fn memory_backend_succeeds() {
        let config = config_with_backend("memory");
        let bus = create_bus(&config);
        assert!(bus.is_ok());
    }

    #[test]
    fn unsupported_backend_errors() {
        let config = config_with_backend("nats");
        let bus = create_bus(&config);
        assert!(bus.is_err());
    }
}
