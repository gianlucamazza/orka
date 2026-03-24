//! Key-value memory store with TTL, search, and compaction.
//!
//! Provides [`RedisMemoryStore`], a Redis implementation of
//! [`orka_core::traits::MemoryStore`], and a [`create_memory_store`] factory.

#![warn(missing_docs)]

/// Redis implementation of the memory store.
pub mod redis_store;

use std::sync::Arc;

use orka_core::{config::primitives::MemoryBackend, traits::MemoryStore};

pub use crate::redis_store::RedisMemoryStore;

/// Create a [`MemoryStore`] from the given configuration.
pub fn create_memory_store(
    config: &orka_core::config::OrkaConfig,
) -> orka_core::Result<Arc<dyn MemoryStore>> {
    match config.memory.backend {
        MemoryBackend::Memory => {
            tracing::debug!(
                max_entries = config.memory.max_entries,
                "in-memory memory store created"
            );
            Ok(Arc::new(orka_core::testing::InMemoryMemoryStore::new()))
        }
        _ => {
            let store = RedisMemoryStore::new(&config.redis.url, config.memory.max_entries)?;
            tracing::debug!(
                max_entries = config.memory.max_entries,
                "memory store created"
            );
            Ok(Arc::new(store))
        }
    }
}

#[cfg(test)]
mod tests {
    use orka_core::config::{
        MemoryConfig,
        primitives::{BusBackend, MemoryBackend},
    };

    /// Mirrors the runtime logic: if memory backend is Auto, follow the bus
    /// backend.
    fn effective_memory_backend(bus: BusBackend, memory: MemoryBackend) -> MemoryBackend {
        if memory == MemoryBackend::Auto {
            match bus {
                BusBackend::Redis => MemoryBackend::Redis,
                BusBackend::Memory => MemoryBackend::Memory,
                BusBackend::Nats => MemoryBackend::Redis,
            }
        } else {
            memory
        }
    }

    #[test]
    fn memory_explicit_memory_overrides_redis_bus() {
        assert_eq!(
            effective_memory_backend(BusBackend::Redis, MemoryBackend::Memory),
            MemoryBackend::Memory
        );
    }

    #[test]
    fn memory_auto_follows_bus_backend() {
        assert_eq!(
            effective_memory_backend(BusBackend::Memory, MemoryBackend::Auto),
            MemoryBackend::Memory
        );
    }

    #[test]
    fn memory_default_backend_is_auto() {
        let config = MemoryConfig::default();
        assert_eq!(config.backend, MemoryBackend::Auto);
    }
}
