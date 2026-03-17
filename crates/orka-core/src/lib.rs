//! Core types, traits, and error handling for the Orka agent orchestration framework.
//!
//! This crate defines the foundational abstractions that all other Orka crates depend on:
//!
//! - **Traits** ([`traits`]): `ChannelAdapter`, `MessageBus`, `SessionStore`, `MemoryStore`,
//!   `PriorityQueue`, `EventSink`, `Skill`, `SecretManager`, `Guardrail`
//! - **Types** ([`types`]): `Envelope`, `OutboundMessage`, `Session`, `Payload`, `DomainEvent`, etc.
//! - **Error** ([`Error`]): unified error type for the entire platform
//! - **Config** ([`config`]): configuration structs loaded from TOML / environment
//! - **Testing** ([`testing`]): in-memory test doubles for all core traits
#![warn(missing_docs)]

#[allow(missing_docs)]
/// Configuration types for the Orka platform.
pub mod config;
/// Unified error type and `Result` alias.
pub mod error;
/// Config versioning and migration engine.
pub mod migrate;
/// Slash-command parser for user input.
pub mod slash_command;
/// Core traits that define the Orka abstraction layer.
pub mod traits;
/// Core data types: envelopes, sessions, events, payloads, and IDs.
pub mod types;

/// Generic retry-with-backoff executor.
pub mod retry;
/// Streaming infrastructure for real-time LLM response delivery.
pub mod stream;

#[allow(missing_docs)]
/// In-memory test doubles for core traits.
pub mod testing;

pub use error::{Error, Result};
pub use slash_command::{ParsedCommand, parse_slash_command};
pub use stream::{StreamChunk, StreamChunkKind, StreamRegistry};
pub use types::{
    CommandPayload, DomainEvent, DomainEventKind, Envelope, EventId, EventPayload, MediaPayload,
    MemoryEntry, MessageId, MessageSink, MessageStream, OutboundMessage, Payload, Priority,
    SecretValue, Session, SessionId, SkillContext, SkillInput, SkillOutput, SkillSchema,
    TraceContext, backoff_delay,
};

/// Prelude for channel adapter implementations.
///
/// ```ignore
/// use orka_core::adapter_prelude::*;
/// ```
pub mod adapter_prelude {
    pub use crate::error::{Error, Result};
    pub use crate::stream::{StreamChunk, StreamChunkKind, StreamRegistry};
    pub use crate::traits::{ChannelAdapter, EventSink};
    pub use crate::types::{
        Envelope, MessageSink, OutboundMessage, Payload, SessionId, backoff_delay,
    };
}
