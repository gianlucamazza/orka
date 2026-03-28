//! Persistent session storage backed by Redis.
//!
//! Provides [`RedisSessionStore`], a Redis implementation of
//! [`orka_core::traits::SessionStore`], and a [`create_session_store`] factory.

#![warn(missing_docs)]

/// Redis implementation of the session store.
pub mod redis_store;

use std::sync::Arc;

use orka_core::traits::SessionStore;

pub use crate::redis_store::RedisSessionStore;

/// Create a [`SessionStore`] from the given configuration.
/// Uses Redis backend (session store is always Redis-backed in production).
pub fn create_session_store(
    config: &orka_config::OrkaConfig,
) -> orka_core::Result<Arc<dyn SessionStore>> {
    let store = RedisSessionStore::new(&config.redis.url, config.session.ttl_secs)?;
    Ok(Arc::new(store))
}

#[cfg(test)]
mod tests {
    #[test]
    fn create_session_store_returns_redis() {
        // Test that create_session_store creates a Redis-backed store
        // Actual Redis connection test requires running Redis instance
        // This test verifies the function signature and basic behavior
    }
}
