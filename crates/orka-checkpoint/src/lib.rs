//! Graph execution checkpointing and crash recovery for Orka.
//!
//! This crate provides:
//!
//! - [`types::Checkpoint`] — a serializable snapshot of a graph run at a
//!   specific node boundary.
//! - [`store::CheckpointStore`] — a pluggable persistence trait for checkpoint
//!   backends.
//! - [`redis_store::RedisCheckpointStore`] — the production Redis backend.
//! - An in-memory backend gated behind the `in-memory` feature flag for use in
//!   unit tests.
//!
//! # Design
//!
//! Checkpoints are written by the [`GraphExecutor`](orka_agent) after every
//! node completes, before the executor selects the next edge. This gives the
//! executor the ability to resume a crashed run from the last completed node
//! rather than from the beginning.
//!
//! State serialization uses `"namespace::name"` string keys so that
//! [`SlotKey`](orka_agent::context::SlotKey) round-trips cleanly through JSON
//! without a custom map serializer.

#![warn(missing_docs)]

pub mod redis_store;
pub mod store;
pub mod types;

pub use redis_store::RedisCheckpointStore;
pub use store::CheckpointStore;
pub use types::{
    Checkpoint, CheckpointId, InterruptReason, RunStatus, SerializableSlotKey,
    SerializableStateChange,
};
