//! Key-value memory store with TTL, search, and compaction.
//!
//! Provides [`RedisMemoryStore`], a Redis implementation of
//! [`orka_core::traits::MemoryStore`], and a [`create_memory_store`] factory.

/// Memory store configuration.
pub mod config;
/// Redis implementation of the memory store.
pub mod redis_store;

use std::sync::Arc;

use orka_core::traits::{MemoryStore, SessionLock};

pub use crate::memory::{
    config::{MemoryBackend, MemoryConfig},
    redis_store::RedisMemoryStore,
};

/// Paired trait objects produced by [`create_memory_store`].
///
/// Both arcs point to the same underlying concrete object, obtained before
/// type erasure so that callers can use history storage and session locking
/// independently.
pub struct MemoryBundle {
    /// Key-value store for conversation history and other memory entries.
    pub store: Arc<dyn MemoryStore>,
    /// Distributed session lock for preventing concurrent history corruption.
    pub lock: Arc<dyn SessionLock>,
}

/// Create a [`MemoryBundle`] from the given configuration.
pub fn create_memory_store(
    config: &MemoryConfig,
    redis_url: &str,
) -> orka_core::Result<MemoryBundle> {
    if config.backend == MemoryBackend::Memory {
        tracing::debug!(
            max_entries = config.max_entries,
            "in-memory memory store created"
        );
        let store = Arc::new(orka_core::testing::InMemoryMemoryStore::new());
        Ok(MemoryBundle {
            lock: Arc::clone(&store) as Arc<dyn SessionLock>,
            store: store as Arc<dyn MemoryStore>,
        })
    } else {
        let store = Arc::new(RedisMemoryStore::new(redis_url, config.max_entries)?);
        tracing::debug!(max_entries = config.max_entries, "memory store created");
        Ok(MemoryBundle {
            lock: Arc::clone(&store) as Arc<dyn SessionLock>,
            store: store as Arc<dyn MemoryStore>,
        })
    }
}
