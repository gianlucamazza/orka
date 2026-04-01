//! Persistent session storage backed by Redis.
//!
//! Provides [`RedisSessionStore`], a Redis implementation of
//! [`orka_core::traits::SessionStore`], and a [`create_session_store`] factory.

/// Session store configuration.
pub mod config;
/// Redis implementation of the session store.
pub mod redis_store;

use std::sync::Arc;

use orka_core::traits::SessionStore;

pub use self::{config::SessionConfig, redis_store::RedisSessionStore};

/// Create a [`SessionStore`] from the given configuration.
/// Uses Redis backend (session store is always Redis-backed in production).
pub fn create_session_store(
    config: &SessionConfig,
    redis_url: &str,
) -> orka_core::Result<Arc<dyn SessionStore>> {
    let store = RedisSessionStore::new(redis_url, config.ttl_secs)?;
    Ok(Arc::new(store))
}
