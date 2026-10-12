//! Redis-backed infrastructure services for Orka.
//!
//! This crate consolidates the message bus, priority queue, session store, and
//! conversation store — all backed by Redis — into a single compilation unit.

#![warn(missing_docs)]

/// Artifact store backed by Redis.
pub mod artifact;
/// Message bus backed by Redis Streams.
pub mod bus;
/// Conversation store backed by Redis.
pub mod conversation;
/// Key-value memory store with TTL, search, and compaction.
pub mod memory;
/// Priority queue backed by Redis Sorted Sets.
pub mod queue;
/// Retry utilities for transient Redis pool errors.
pub mod retry;
/// Secret storage backends (Redis-backed and file-backed, optional AES-256-GCM
/// encryption).
pub mod secrets;
/// Session store backed by Redis.
pub mod session;

/// Create a `deadpool-redis` connection pool for the given Redis URL.
///
/// redis 1.x defaults `response_timeout` to 500ms, which aborts blocking
/// commands (`BZPOPMIN`, `XREADGROUP BLOCK`) before the server wait finishes.
/// Disable that client timeout so the command-level timeout is authoritative.
pub(crate) fn create_redis_pool(
    redis_url: &str,
) -> Result<deadpool_redis::Pool, Box<dyn std::error::Error + Send + Sync>> {
    let conn_config = redis::AsyncConnectionConfig::new().set_response_timeout(None);
    let manager = deadpool_redis::Manager::new_with_config(redis_url, conn_config)?;
    Ok(deadpool_redis::Pool::builder(manager)
        .runtime(deadpool_redis::Runtime::Tokio1)
        .build()?)
}

// Flat re-exports for backwards-compatible access.
pub use artifact::{RedisArtifactStore, create_artifact_store};
pub use bus::{BusBackend, BusConfig, RedisBus, create_bus};
pub use conversation::{RedisConversationStore, create_conversation_store};
pub use memory::{
    MemoryBackend, MemoryBundle, MemoryConfig, RedisMemoryStore, create_memory_store,
};
pub use queue::{QueueBundle, RedisPriorityQueue, create_queue, priority_score};
pub use secrets::{
    FileSecretManager, RedisSecretManager, RotatingSecretManager, RotationConfig, RotationStatus,
    SecretBackend, SecretConfig, create_file_secret_manager, create_secret_manager,
    default_secrets_file_path,
};
pub use session::{RedisSessionStore, SessionConfig, create_session_store};
