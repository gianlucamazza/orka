//! Capability model for Orka integrations.
//!
//! Each integration declares the set of capabilities it supports via
//! [`CapabilitySet`]. This replaces implicit, undocumented parities with an
//! explicit, testable contract.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// A discrete capability that an integration surface may support.
///
/// Capabilities are declared by adapters and used by orchestration to decide
/// whether a given feature can be used on a channel, to degrade gracefully
/// when a capability is absent, and to expose integration metadata via the API.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    utoipa::ToSchema,
)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Can receive text messages from users.
    TextInbound,
    /// Can send text messages to users.
    TextOutbound,
    /// Supports streaming LLM output as incremental deltas.
    StreamingDeltas,
    /// Can edit previously sent messages.
    MessageEdit,
    /// Can show typing/composing indicators.
    TypingIndicator,
    /// Supports slash commands or structured commands.
    SlashCommands,
    /// Supports interactive callbacks (button clicks, inline keyboards, etc.).
    InteractiveCallbacks,
    /// Supports threaded conversations or reply chains.
    Threading,
    /// Can receive media/file attachments from users.
    MediaInbound,
    /// Can send media/file attachments to users.
    MediaOutbound,
    /// Exposes conversation control operations (cancel, retry, delete, mark
    /// read).
    ConversationControl,
    /// Supports device/client pairing flows.
    SessionPairing,
    /// Supports device identity and authentication.
    DeviceAuth,
    /// Supports artifact lifecycle (upload, attach, detach).
    ArtifactLifecycle,
    /// Supports direct file upload by users.
    FileUpload,
    /// Supports rich text formatting (bold, italic, code, etc.).
    RichText,
    /// Receives messages via webhook push (platform calls us).
    WebhookPush,
    /// Bidirectional real-time communication via WebSocket.
    WebsocketBidirectional,
}

/// The set of capabilities declared by an integration.
///
/// Uses [`BTreeSet`] for deterministic ordering, readable debug output, and
/// natural JSON array serialization. Extensible without bit-width limits.
pub type CapabilitySet = BTreeSet<Capability>;
