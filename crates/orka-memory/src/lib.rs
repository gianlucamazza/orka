//! Key-value memory store with TTL, search, and compaction.
//!
//! Provides [`RedisMemoryStore`], a Redis implementation of
//! [`orka_core::traits::MemoryStore`], and a [`create_memory_store`] factory.

#![warn(missing_docs)]

#[allow(missing_docs)]
pub mod redis_store;

pub use crate::redis_store::RedisMemoryStore;

use std::sync::Arc;

use orka_core::traits::MemoryStore;

/// Create a [`MemoryStore`] from the given configuration.
pub fn create_memory_store(
    config: &orka_core::config::OrkaConfig,
) -> orka_core::Result<Arc<dyn MemoryStore>> {
    match config.bus.backend.as_str() {
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
