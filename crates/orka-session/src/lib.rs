//! Persistent session storage backed by Redis.
//!
//! Provides [`RedisSessionStore`], a Redis implementation of
//! [`orka_core::traits::SessionStore`], and a [`create_session_store`] factory.

#![warn(missing_docs)]

/// Redis implementation of the session store.
pub mod redis_store;

pub use crate::redis_store::RedisSessionStore;

use std::sync::Arc;

use orka_core::traits::SessionStore;

/// Create a [`SessionStore`] from the given configuration.
pub fn create_session_store(
    config: &orka_core::config::OrkaConfig,
) -> orka_core::Result<Arc<dyn SessionStore>> {
    let effective = if config.session.backend == "auto" {
        config.bus.backend.as_str()
    } else {
        config.session.backend.as_str()
    };
    match effective {
        "memory" => Ok(Arc::new(orka_core::testing::InMemorySessionStore::new())),
        _ => {
            let store = RedisSessionStore::new(&config.redis.url, config.session.ttl_secs)?;
            Ok(Arc::new(store))
        }
    }
}

#[cfg(test)]
mod tests {
    use orka_core::config::{BusConfig, SessionConfig};

    fn effective_backend(bus_backend: &str, session_backend: &str) -> String {
        let bus = BusConfig {
            backend: bus_backend.into(),
            ..Default::default()
        };
        let session = SessionConfig {
            backend: session_backend.into(),
            ..Default::default()
        };
        if session.backend == "auto" {
            bus.backend.clone()
        } else {
            session.backend.clone()
        }
    }

    #[test]
    fn session_explicit_memory_overrides_redis_bus() {
        assert_eq!(effective_backend("redis", "memory"), "memory");
    }

    #[test]
    fn session_auto_follows_bus_backend() {
        assert_eq!(effective_backend("memory", "auto"), "memory");
    }

    #[test]
    fn session_default_backend_is_auto() {
        let config = SessionConfig::default();
        assert_eq!(config.backend, "auto");
    }
}
