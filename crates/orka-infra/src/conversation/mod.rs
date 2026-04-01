//! Conversation storage backed by Redis.
//!
//! Provides [`RedisConversationStore`] and a factory for wiring it into the
//! server runtime.

/// Redis implementation of the conversation store.
pub mod redis_store;

use std::sync::Arc;

use orka_core::traits::ConversationStore;

pub use self::redis_store::RedisConversationStore;

/// Create a [`ConversationStore`] from the given Redis URL.
pub fn create_conversation_store(redis_url: &str) -> orka_core::Result<Arc<dyn ConversationStore>> {
    let store = RedisConversationStore::new(redis_url)?;
    Ok(Arc::new(store))
}
