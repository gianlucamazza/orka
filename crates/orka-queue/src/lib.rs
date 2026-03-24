//! Priority queue for ordered message processing.
//!
//! Provides [`RedisPriorityQueue`], a Redis sorted-set implementation of
//! [`orka_core::traits::PriorityQueue`], and a [`create_queue`] factory.

#![warn(missing_docs)]

/// Redis sorted-set implementation of the priority queue.
pub mod redis_queue;

use std::sync::Arc;

use orka_core::{Result, config::OrkaConfig, traits::PriorityQueue};
pub use redis_queue::RedisPriorityQueue;

/// Create a priority queue from the given configuration.
/// Uses Redis backend (queue is always Redis-backed in production).
pub fn create_queue(config: &OrkaConfig) -> Result<Arc<dyn PriorityQueue>> {
    let queue = RedisPriorityQueue::new(&config.redis.url)?;
    Ok(Arc::new(queue))
}

#[cfg(test)]
mod tests {
    #[test]
    fn create_queue_returns_redis() {
        // Test that create_queue creates a Redis-backed queue
        // Actual Redis connection test requires running Redis instance
    }
}
