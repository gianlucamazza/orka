//! Core data types for the Orka agent orchestration framework.
//!
//! Organised into focused sub-modules by domain:
//!
//! | Module | Contents |
//! |--------|----------|
//! | [`ids`] | Typed ID wrappers (UUID v7) and [`ErrorCategory`] |
//! | [`events`] | [`DomainEvent`] and [`DomainEventKind`] |
//! | [`envelope`] | [`Envelope`], [`Payload`], [`OutboundMessage`], [`MediaPayload`], etc. |
//! | [`skill`] | [`SkillInput`], [`SkillOutput`], [`SkillContext`], [`SkillSchema`] |
//! | [`session`] | [`Session`] |
//! | [`conversation`] | [`Conversation`], [`ConversationMessage`], [`ConversationArtifact`] |
//! | [`memory`] | [`MemoryEntry`], [`MemoryKind`], [`MemoryScope`] |
//! | [`secrets`] | [`SecretStr`], [`SecretValue`] |
//! | [`aliases`] | Type aliases (`MessageSink`, `SessionCancelTokens`, etc.) |

mod aliases;
mod conversation;
mod envelope;
mod events;
mod ids;
mod memory;
mod secrets;
mod session;
mod skill;

pub use aliases::*;
pub use conversation::*;
pub use envelope::*;
pub use events::*;
pub use ids::*;
pub use memory::*;
pub use secrets::*;
pub use session::*;
pub use skill::*;
