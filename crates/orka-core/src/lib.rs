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
/// Slash-command parser for user input.
pub mod slash_command;
/// Core traits that define the Orka abstraction layer.
pub mod traits;
/// Core data types: envelopes, sessions, events, payloads, and IDs.
pub mod types;

#[allow(missing_docs)]
/// In-memory test doubles for core traits.
pub mod testing;

pub use error::{Error, Result};
pub use slash_command::{parse_slash_command, ParsedCommand};
pub use types::*;
