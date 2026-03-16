pub mod redis_queue;

pub use redis_queue::RedisPriorityQueue;

use orka_core::{config::OrkaConfig, traits::PriorityQueue};
use std::sync::Arc;

/// Create a priority queue from the given configuration.
pub fn create_queue(config: &OrkaConfig) -> Arc<dyn PriorityQueue> {
    match config.bus.backend.as_str() {
        "memory" => Arc::new(orka_core::testing::InMemoryQueue::new()),
        _ => {
            let queue = RedisPriorityQueue::new(&config.redis.url)
                .expect("failed to create Redis priority queue");
            Arc::new(queue)
        }
    }
}
