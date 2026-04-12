//! Core types, traits, and error handling for the Orka agent orchestration
//! framework.
//!
//! This crate defines the foundational abstractions that all other Orka crates
//! depend on:
//!
//! - **Traits** ([`traits`]): `ChannelAdapter`, `MessageBus`, `SessionStore`,
//!   `ConversationStore`, `MemoryStore`, `PriorityQueue`, `EventSink`, `Skill`,
//!   `SecretManager`, `Guardrail`
//! - **Types** ([`types`]): `Envelope`, `OutboundMessage`, `Session`,
//!   `Conversation`, `Payload`, `DomainEvent`, etc.
//! - **Error** ([`Error`]): unified error type for the entire platform
//! - **Config** ([`config`]): shared config submodules and primitives used by
//!   the composed workspace config
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

/// Unified error type and `Result` alias.
pub mod error;
/// Slash-command parser for user input.
pub(crate) mod slash_command;
/// Core traits that define the Orka abstraction layer.
pub mod traits;
/// Core data types: envelopes, sessions, events, payloads, and IDs.
pub mod types;

/// Generic retry-with-backoff executor.
pub(crate) mod retry;
/// Streaming infrastructure for real-time LLM response delivery.
pub mod stream;
/// Shared utility functions (e.g., string helpers).
pub(crate) mod util;

/// Canonical contracts: capability model, interaction types, platform context,
/// and realtime event schema.
pub mod contracts;

/// Configuration types for the Orka platform.
#[cfg(feature = "config")]
pub mod config;

/// In-memory test doubles for core traits.
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use contracts::{
    Capability, CapabilitySet, CommandContent, EventContent, InboundInteraction, IntegrationClass,
    InteractionContent, MediaAttachment, OutboundInteraction, PlatformContext, RealtimeEvent,
    RichInput, SenderInfo, TraceContext, TrustLevel,
};
pub use error::{Error, Result};
pub use retry::retry_with_backoff;
pub use slash_command::{ParsedCommand, parse_slash_command};
pub use stream::{StreamChunk, StreamChunkKind, StreamRegistry, forward_delegate_progress};
pub use traits::{MessageCursor, NoopEventSink, SearchHit, apply_message_cursors, extract_snippet};
pub use types::{
    ArtifactId, CommandArgs, CommandPayload, Conversation, ConversationArtifact,
    ConversationArtifactOrigin, ConversationId, ConversationMessage, ConversationMessageRole,
    ConversationMessageStatus, ConversationStatus, DomainEvent, DomainEventKind, Envelope,
    ErrorCategory, EventId, EventPayload, InteractionSink, MediaPayload, MemoryEntry, MemoryKind,
    MemoryScope, MessageId, MessageSink, MessageStream, OutboundMessage, Payload, PrincipleKind,
    Priority, RichInputPayload, RunId, SecretStr, SecretValue, Session, SessionCancelTokens,
    SessionId, SkillBudget, SkillContext, SkillInput, SkillOutput, SkillSchema,
    SoftSkillSelectionMode, backoff_delay,
};
pub use util::truncate_tool_result;
