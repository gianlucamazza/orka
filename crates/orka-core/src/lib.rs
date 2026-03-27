//! Core types, traits, and error handling for the Orka agent orchestration
//! framework.
//!
//! This crate defines the foundational abstractions that all other Orka crates
//! depend on:
//!
//! - **Traits** ([`traits`]): `ChannelAdapter`, `MessageBus`, `SessionStore`,
//!   `MemoryStore`, `PriorityQueue`, `EventSink`, `Skill`, `SecretManager`,
//!   `Guardrail`
//! - **Types** ([`types`]): `Envelope`, `OutboundMessage`, `Session`,
//!   `Payload`, `DomainEvent`, etc.
//! - **Error** ([`Error`]): unified error type for the entire platform
//! - **Config** ([`config`]): configuration structs loaded from TOML /
//!   environment
//! - **Container** ([`container`]): lightweight dependency injection container
//! - **Testing** ([`testing`]): in-memory test doubles for all core traits
//!
//! # Examples
//!
//! ```
//! use orka_core::{Error, Result};
//!
//! fn might_fail(ok: bool) -> Result<String> {
//!     if ok {
//!         Ok("success".into())
//!     } else {
//!         Err(Error::Other("something went wrong".into()))
//!     }
//! }
//!
//! assert!(might_fail(true).is_ok());
//! assert!(might_fail(false).is_err());
//! ```
#![warn(missing_docs)]

/// Lightweight dependency injection container.
pub mod container;
/// Unified error type and `Result` alias.
pub mod error;
/// Slash-command parser for user input.
pub mod slash_command;
/// Core traits that define the Orka abstraction layer.
pub mod traits;
/// Core data types: envelopes, sessions, events, payloads, and IDs.
pub mod types;

/// Progress bridge: forwards coding-delegate events to chat platforms.
pub mod progress_bridge;
/// Generic retry-with-backoff executor.
pub mod retry;
/// Streaming infrastructure for real-time LLM response delivery.
pub mod stream;
/// Shared utility functions (e.g., string helpers).
pub mod util;

/// Configuration types for the Orka platform.
#[cfg(feature = "config")]
pub mod config;

/// Config versioning and migration engine.
#[cfg(feature = "migrate")]
pub mod migrate;

/// In-memory test doubles for core traits.
#[cfg(feature = "testing")]
pub mod testing;

#[cfg(feature = "config")]
pub use config::OrkaConfig;
pub use error::{Error, Result};
#[cfg(feature = "migrate")]
pub use migrate::{
    MigrationError, MigrationResult, inspect_config_issues, migrate_for_write, migrate_if_needed,
};
pub use slash_command::{ParsedCommand, parse_slash_command};
pub use stream::{StreamChunk, StreamChunkKind, StreamRegistry, forward_delegate_progress};
pub use traits::NoopEventSink;
pub use types::{
    CommandArgs, CommandPayload, DomainEvent, DomainEventKind, Envelope, ErrorCategory, EventId,
    EventPayload, MediaPayload, MemoryEntry, MemoryKind, MemoryScope, MessageId, MessageSink,
    MessageStream, OutboundMessage, Payload, Priority, RunId, SecretValue, Session, SessionId,
    SkillBudget, SkillContext, SkillInput, SkillOutput, SkillSchema, TraceContext, backoff_delay,
};
pub use util::truncate_tool_result;
