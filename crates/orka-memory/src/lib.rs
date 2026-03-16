pub mod redis_store;

pub use crate::redis_store::RedisMemoryStore;

use std::sync::Arc;

use orka_core::traits::MemoryStore;

pub fn create_memory_store(
    config: &orka_core::config::OrkaConfig,
) -> orka_core::Result<Arc<dyn MemoryStore>> {
    match config.bus.backend.as_str() {
        "memory" => {
            tracing::debug!(max_entries = config.memory.max_entries, "in-memory memory store created");
            Ok(Arc::new(orka_core::testing::InMemoryMemoryStore::new()))
        }
        _ => {
            let store = RedisMemoryStore::new(&config.redis.url)?;
            tracing::debug!(max_entries = config.memory.max_entries, "memory store created");
            Ok(Arc::new(store))
        }
    }
}
