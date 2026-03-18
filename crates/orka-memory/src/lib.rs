//! Key-value memory store with TTL, search, and compaction.
//!
//! Provides [`RedisMemoryStore`], a Redis implementation of
//! [`orka_core::traits::MemoryStore`], and a [`create_memory_store`] factory.

#![warn(missing_docs)]

/// Redis implementation of the memory store.
pub mod redis_store;

pub use crate::redis_store::RedisMemoryStore;

use std::sync::Arc;

use orka_core::traits::MemoryStore;

/// Create a [`MemoryStore`] from the given configuration.
pub fn create_memory_store(
    config: &orka_core::config::OrkaConfig,
) -> orka_core::Result<Arc<dyn MemoryStore>> {
    let effective = if config.memory.backend == "auto" {
        config.bus.backend.as_str()
    } else {
        config.memory.backend.as_str()
    };
    match effective {
        "memory" => {
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
    use orka_core::config::{BusConfig, MemoryConfig};

    fn effective_backend(bus_backend: &str, memory_backend: &str) -> String {
        let bus = BusConfig {
            backend: bus_backend.into(),
            ..Default::default()
        };
        let memory = MemoryConfig {
            backend: memory_backend.into(),
            ..Default::default()
        };
        if memory.backend == "auto" {
            bus.backend.clone()
        } else {
            memory.backend.clone()
        }
    }

    #[test]
    fn memory_explicit_memory_overrides_redis_bus() {
        assert_eq!(effective_backend("redis", "memory"), "memory");
    }

    #[test]
    fn memory_auto_follows_bus_backend() {
        assert_eq!(effective_backend("memory", "auto"), "memory");
    }

    #[test]
    fn memory_default_backend_is_auto() {
        let config = MemoryConfig::default();
        assert_eq!(config.backend, "auto");
    }
}
