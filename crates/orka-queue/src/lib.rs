//! Priority queue for ordered message processing.
//!
//! Provides [`RedisPriorityQueue`], a Redis sorted-set implementation of
//! [`orka_core::traits::PriorityQueue`], and a [`create_queue`] factory.

#![warn(missing_docs)]

/// Redis sorted-set implementation of the priority queue.
pub mod redis_queue;

use std::sync::Arc;

use orka_config::OrkaConfig;
use orka_core::{
    Result,
    traits::{DeadLetterQueue, PriorityQueue},
};
pub use redis_queue::RedisPriorityQueue;

/// Paired trait objects produced by [`create_queue`].
///
/// Both arcs point to the same underlying concrete object, obtained before
/// type erasure so that callers can use the main queue and DLQ independently.
pub struct QueueBundle {
    /// Priority queue for normal message processing.
    pub queue: Arc<dyn PriorityQueue>,
    /// Dead-letter queue for messages that exhausted all retry attempts.
    pub dlq: Arc<dyn DeadLetterQueue>,
}

/// Create a [`QueueBundle`] from the given configuration.
/// Uses Redis backend (queue is always Redis-backed in production).
pub fn create_queue(config: &OrkaConfig) -> Result<QueueBundle> {
    let queue = Arc::new(RedisPriorityQueue::new(&config.redis.url)?);
    Ok(QueueBundle {
        dlq: Arc::clone(&queue) as Arc<dyn DeadLetterQueue>,
        queue: queue as Arc<dyn PriorityQueue>,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn create_queue_returns_redis() {
        // Test that create_queue creates a Redis-backed queue
        // Actual Redis connection test requires running Redis instance
    }
}
