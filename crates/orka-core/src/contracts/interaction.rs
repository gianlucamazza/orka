//! Canonical interaction types: the public contract for inbound and outbound
//! messages across all integration surfaces.
//!
//! Adapters produce [`InboundInteraction`] from platform-specific events.
//! The bridge layer in the server converts to the internal [`Envelope`] type
//! for the message bus. Outbound, the worker produces platform-agnostic
//! [`OutboundInteraction`] that adapters render to their respective protocols.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::platform::PlatformContext;

/// W3C Trace Context propagation headers.
///
/// Not `#[non_exhaustive]` — adapters must be able to construct this directly.
/// New optional fields are added without a major version bump by keeping them
/// `Option<_>` so existing adapter code compiles unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TraceContext {
    /// W3C traceparent `trace-id` component (32 lowercase hex characters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    /// W3C traceparent `parent-id` component (16 lowercase hex characters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
    /// W3C trace flags byte (`1` = sampled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_flags: Option<u8>,
}

/// The content of an inbound or outbound interaction.
///
/// Mirrors the internal `Payload` enum but belongs to the contracts module so
/// adapters can depend on it without extra complexity. The bridge layer
/// converts between these two types at the single integration boundary.
///
/// `#[non_exhaustive]` because new content kinds may be added. Match arms
/// in external crates must include a wildcard `_` arm.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum InteractionContent {
    /// Plain text message.
    Text(String),
    /// Rich input combining optional text and media attachments.
    RichInput(RichInput),
    /// A single media or file attachment.
    Media(MediaAttachment),
    /// A structured slash command.
    Command(CommandContent),
    /// A lifecycle or system event.
    Event(EventContent),
}

/// Rich user input with optional text and one or more attachments.
///
/// Not `#[non_exhaustive]` — adapters construct this directly.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RichInput {
    /// Optional accompanying text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Media attachments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<MediaAttachment>,
}

/// A media or file attachment.
///
/// Not `#[non_exhaustive]` — adapters construct this directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaAttachment {
    /// MIME type (e.g. `"image/png"`, `"audio/ogg"`).
    pub mime_type: String,
    /// URL or path where the content can be retrieved.
    pub url: String,
    /// Suggested filename.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// Optional description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    /// File size in bytes, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    /// Inline content encoded as standard base64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_base64: Option<String>,
}

/// A structured command invocation.
///
/// Not `#[non_exhaustive]` — adapters construct this directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandContent {
    /// Command name (without the leading slash).
    pub name: String,
    /// Named arguments.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub args: HashMap<String, Value>,
}

/// A system or lifecycle event.
///
/// Not `#[non_exhaustive]` — adapters construct this directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventContent {
    /// Short string identifier for the event kind.
    pub kind: String,
    /// Arbitrary event payload.
    pub data: Value,
}

/// A canonical inbound interaction produced by an adapter.
///
/// This is the single entry point for all external messages into Orka. Adapters
/// produce this type from their platform-specific event model. The bridge in
/// `orka-server` converts it to an internal `Envelope` for the message bus.
///
/// Sender identity is carried inside `context.sender` so that all platform
/// metadata — routing and identity — travels together through the pipeline.
///
/// Not `#[non_exhaustive]` — adapters must construct this directly. New fields
/// are introduced as `Option<_>` with `#[serde(default)]` to avoid breaking
/// existing adapter code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundInteraction {
    /// Unique ID for this interaction (UUID v7).
    pub id: Uuid,
    /// The channel that originated this interaction (e.g. `"telegram"`).
    pub source_channel: String,
    /// Session identifier (UUID v7).
    pub session_id: Uuid,
    /// When the interaction was created.
    pub timestamp: DateTime<Utc>,
    /// The interaction content.
    pub content: InteractionContent,
    /// Platform routing context, including sender identity (`context.sender`).
    pub context: PlatformContext,
    /// Distributed tracing propagation.
    pub trace: TraceContext,
}

/// A canonical outbound interaction produced by the worker.
///
/// Adapters render this to their platform-specific wire format. Only fields
/// the adapter actually needs are consumed; the rest are ignored.
///
/// Not `#[non_exhaustive]` — adapters must be able to inspect all fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundInteraction {
    /// Target channel identifier.
    pub channel: String,
    /// Session identifier (UUID v7).
    pub session_id: Uuid,
    /// The content to deliver.
    pub content: InteractionContent,
    /// The inbound interaction this is a reply to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<Uuid>,
    /// Platform routing context (carries `chat_id`, etc. for delivery).
    pub context: PlatformContext,
}
