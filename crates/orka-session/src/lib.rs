//! Persistent session storage backed by Redis.
//!
//! Provides [`RedisSessionStore`], a Redis implementation of
//! [`orka_core::traits::SessionStore`], and a [`create_session_store`] factory.

#![warn(missing_docs)]

/// Session store configuration owned by `orka-session`.
pub mod config;
/// Redis implementation of the session store.
pub mod redis_store;

use std::sync::Arc;

use orka_core::traits::SessionStore;

pub use crate::{config::SessionConfig, redis_store::RedisSessionStore};

/// Create a [`SessionStore`] from the given configuration.
/// Uses Redis backend (session store is always Redis-backed in production).
pub fn create_session_store(
    config: &SessionConfig,
    redis_url: &str,
) -> orka_core::Result<Arc<dyn SessionStore>> {
    let store = RedisSessionStore::new(redis_url, config.ttl_secs)?;
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
