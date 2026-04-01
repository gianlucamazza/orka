//! Redis-backed infrastructure services for Orka.
//!
//! This crate consolidates the message bus, priority queue, session store, and
//! conversation store — all backed by Redis — into a single compilation unit.

#![warn(missing_docs)]

/// Message bus backed by Redis Streams.
pub mod bus;
/// Priority queue backed by Redis Sorted Sets.
pub mod queue;
/// Session store backed by Redis.
pub mod session;
/// Conversation store backed by Redis.
pub mod conversation;

// Flat re-exports for backwards-compatible access.
pub use bus::{BusBackend, BusConfig, RedisBus, create_bus};
pub use conversation::{RedisConversationStore, create_conversation_store};
pub use queue::{QueueBundle, RedisPriorityQueue, create_queue, priority_score};
pub use session::{RedisSessionStore, SessionConfig, create_session_store};
