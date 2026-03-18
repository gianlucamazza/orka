//! Publish/subscribe message bus backed by Redis Streams.
//!
//! Provides the [`RedisBus`] implementation of [`orka_core::traits::MessageBus`]
//! and a [`create_bus`] factory that selects the backend from configuration.

#![warn(missing_docs)]

/// Redis Streams implementation of the message bus.
pub mod redis_bus;

pub use crate::redis_bus::RedisBus;

use std::sync::Arc;

use orka_core::traits::MessageBus;

/// Create a [`MessageBus`] from the given configuration.
pub fn create_bus(
    config: &orka_core::config::OrkaConfig,
) -> orka_core::Result<Arc<dyn MessageBus>> {
    match config.bus.backend.as_str() {
        "redis" => {
            let bus = RedisBus::new(&config.redis.url, &config.bus)?;
            Ok(Arc::new(bus))
        }
        "memory" => Ok(Arc::new(orka_core::testing::InMemoryBus::new())),
        other => Err(orka_core::Error::bus(format!(
            "unsupported bus backend: {other}"
        ))),
    }
}
