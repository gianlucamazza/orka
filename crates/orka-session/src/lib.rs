//! Persistent session storage backed by Redis.
//!
//! Provides [`RedisSessionStore`], a Redis implementation of
//! [`orka_core::traits::SessionStore`], and a [`create_session_store`] factory.

#![warn(missing_docs)]

#[allow(missing_docs)]
pub mod redis_store;

pub use crate::redis_store::RedisSessionStore;

use std::sync::Arc;

use orka_core::traits::SessionStore;

/// Create a [`SessionStore`] from the given configuration.
pub fn create_session_store(
    config: &orka_core::config::OrkaConfig,
) -> orka_core::Result<Arc<dyn SessionStore>> {
    match config.bus.backend.as_str() {
        "memory" => Ok(Arc::new(orka_core::testing::InMemorySessionStore::new())),
        _ => {
            let store = RedisSessionStore::new(&config.redis.url, config.session.ttl_secs)?;
            Ok(Arc::new(store))
        }
    }
}
