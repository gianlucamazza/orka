//! Priority queue for ordered message processing.
//!
//! Provides [`RedisPriorityQueue`], a Redis sorted-set implementation of
//! [`orka_core::traits::PriorityQueue`], and a [`create_queue`] factory.

#![warn(missing_docs)]

/// Redis sorted-set implementation of the priority queue.
pub mod redis_queue;

pub use redis_queue::RedisPriorityQueue;

use orka_core::{Result, config::OrkaConfig, traits::PriorityQueue};
use std::sync::Arc;

/// Create a priority queue from the given configuration.
pub fn create_queue(config: &OrkaConfig) -> Result<Arc<dyn PriorityQueue>> {
    let effective = if config.queue.backend == "auto" {
        config.bus.backend.as_str()
    } else {
        config.queue.backend.as_str()
    };
    match effective {
        "memory" => Ok(Arc::new(orka_core::testing::InMemoryQueue::new())),
        _ => {
            let queue = RedisPriorityQueue::new(&config.redis.url)?;
            Ok(Arc::new(queue))
        }
    }
}

#[cfg(test)]
mod tests {
    use orka_core::config::{BusConfig, QueueConfig};

    fn effective_backend(bus_backend: &str, queue_backend: &str) -> String {
        let bus = BusConfig {
            backend: bus_backend.into(),
            ..Default::default()
        };
        let queue = QueueConfig {
            backend: queue_backend.into(),
            ..Default::default()
        };
        if queue.backend == "auto" {
            bus.backend.clone()
        } else {
            queue.backend.clone()
        }
    }

    #[test]
    fn queue_explicit_memory_overrides_redis_bus() {
        assert_eq!(effective_backend("redis", "memory"), "memory");
    }

    #[test]
    fn queue_auto_follows_bus_backend() {
        assert_eq!(effective_backend("memory", "auto"), "memory");
    }

    #[test]
    fn queue_default_backend_is_auto() {
        let config = QueueConfig::default();
        assert_eq!(config.backend, "auto");
    }
}
