pub mod redis_bus;

#[cfg(feature = "bus-nats")]
pub mod nats;

pub use crate::redis_bus::RedisBus;

use std::sync::Arc;

use orka_core::traits::MessageBus;

pub fn create_bus(config: &orka_core::config::OrkaConfig) -> orka_core::Result<Arc<dyn MessageBus>> {
    match config.bus.backend.as_str() {
        "redis" => {
            let bus = RedisBus::new(&config.redis.url)?;
            Ok(Arc::new(bus))
        }
        "memory" => {
            Ok(Arc::new(orka_core::testing::InMemoryBus::new()))
        }
        other => Err(orka_core::Error::bus(format!(
            "unsupported bus backend: {other}"
        ))),
    }
}
