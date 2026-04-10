use std::collections::HashMap;

use chrono::{DateTime, Utc};
use orka_contracts::{
    CommandContent, EventContent, InboundInteraction, InteractionContent, MediaAttachment,
    PlatformContext, RichInput, TraceContext,
};
use serde::{Deserialize, Serialize};

use super::ids::{MessageId, SessionId};

/// Product-facing rich input payload combining text with media attachments.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RichInputPayload {
    /// Optional user-authored text accompanying the attachments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Media attachments submitted in the same turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<MediaPayload>,
}

/// Message priority for queue routing.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
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
pub enum Priority {
    /// Lowest priority; processed after Normal and Urgent messages.
    Background = 0,
    /// Default priority for standard messages.
    #[default]
    Normal = 1,
    /// Highest priority, used for direct messages and time-sensitive work.
    Urgent = 2,
}

/// Message payload variants.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
#[serde(tag = "type", content = "data")]
pub enum Payload {
    /// Plain text message content.
    Text(String),
    /// Rich user input combining text and attachments in a single turn.
    RichInput(RichInputPayload),
    /// File or media attachment.
    Media(MediaPayload),
    /// Structured slash command from a user or internal system.
    Command(CommandPayload),
    /// Internal system or lifecycle event.
    Event(EventPayload),
}

/// Media attachment info.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct MediaPayload {
    /// MIME type of the media content (e.g. `image/png`, `audio/ogg`).
    pub mime_type: String,
    /// URL or path where the media can be retrieved. Empty for inline payloads.
    pub url: String,
    /// Suggested filename when materialized as a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// Optional human-readable description of the media.
    pub caption: Option<String>,
    /// File size in bytes, if known.
    pub size_bytes: Option<u64>,
    /// Inline media data encoded as standard base64. When present, adapters use
    /// this directly (multipart upload) instead of fetching from `url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_base64: Option<String>,
}

impl MediaPayload {
    /// Create a new media payload referencing an external URL.
    pub fn new(mime_type: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            mime_type: mime_type.into(),
            url: url.into(),
            filename: None,
            caption: None,
            size_bytes: None,
            data_base64: None,
        }
    }

    /// Create an inline media payload from raw bytes.
    ///
    /// The data is base64-encoded and stored in `data_base64`; `url` is left
    /// empty. Adapters that support multipart upload use the inline data
    /// directly without making an outbound HTTP request.
    pub fn inline(
        mime_type: impl Into<String>,
        data: Vec<u8>,
        caption: impl Into<Option<String>>,
    ) -> Self {
        use base64::Engine as _;
        let size = data.len() as u64;
        Self {
            mime_type: mime_type.into(),
            url: String::new(),
            filename: None,
            caption: caption.into(),
            size_bytes: Some(size),
            data_base64: Some(base64::engine::general_purpose::STANDARD.encode(data)),
        }
    }

    /// Decode the inline base64 data, if present.
    pub fn decode_data(&self) -> Option<Vec<u8>> {
        use base64::Engine as _;
        self.data_base64
            .as_deref()
            .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
    }

    /// Set the suggested filename for this payload.
    #[must_use]
    pub fn with_filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }
}

/// Structured command from a channel or internal system.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct CommandPayload {
    /// The command name (without the leading slash).
    pub name: String,
    /// Named parameters parsed from the command invocation.
    pub args: HashMap<String, serde_json::Value>,
}

impl CommandPayload {
    /// Create a new command payload.
    pub fn new(name: impl Into<String>, args: HashMap<String, serde_json::Value>) -> Self {
        Self {
            name: name.into(),
            args,
        }
    }
}

/// Unified command arguments produced by any adapter.
///
/// Both structured adapters (Discord slash commands with typed options) and
/// text-based adapters (Telegram `bot_command` entity) normalise their input
/// into this type.  This eliminates the round-trip `CommandPayload → text →
/// re-parse` that previously happened in the worker.
///
/// Constructed via [`From<CommandPayload>`] or [`From<crate::ParsedCommand>`].
#[derive(Debug, Clone, Default)]
pub struct CommandArgs {
    /// Positional tokens: everything that is *not* a `key=value` pair.
    positional: Vec<String>,
    /// Named parameters parsed from `key=value` tokens.
    named: HashMap<String, serde_json::Value>,
    /// The raw text argument string, if available (used by [`Self::text`]).
    raw: Option<String>,
}

impl CommandArgs {
    /// All positional argument tokens.
    pub fn positional_args(&self) -> &[String] {
        &self.positional
    }

    /// The n-th positional argument, or `None` if out of range.
    pub fn positional(&self, i: usize) -> Option<&str> {
        self.positional.get(i).map(String::as_str)
    }

    /// The raw text following the command name, or `None` if there were no
    /// arguments.
    ///
    /// Equivalent to all positional tokens joined by a single space when no raw
    /// string was preserved.
    pub fn text(&self) -> Option<&str> {
        if self.positional.is_empty() && self.named.is_empty() {
            return None;
        }
        self.raw.as_deref()
    }

    /// A named argument value, or `None` if not present.
    pub fn named(&self, key: &str) -> Option<&serde_json::Value> {
        self.named.get(key)
    }

    /// Iterate over all named `(key, value)` pairs.
    pub fn named_iter(&self) -> impl Iterator<Item = (&str, &serde_json::Value)> {
        self.named.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// `true` if there are no positional or named arguments.
    pub fn is_empty(&self) -> bool {
        self.positional.is_empty() && self.named.is_empty()
    }
}

fn split_args(tokens: Vec<String>) -> (Vec<String>, HashMap<String, serde_json::Value>) {
    let mut positional = Vec::new();
    let mut named = HashMap::new();
    for token in tokens {
        if let Some((k, v)) = token.split_once('=') {
            let value = serde_json::from_str(v)
                .unwrap_or_else(|_| serde_json::Value::String(v.to_string()));
            named.insert(k.to_string(), value);
        } else {
            positional.push(token);
        }
    }
    (positional, named)
}

impl From<CommandPayload> for CommandArgs {
    fn from(cmd: CommandPayload) -> Self {
        if let Some(raw_text) = cmd.args.get("text").and_then(|v| v.as_str()) {
            let raw = raw_text.to_string();
            let tokens = crate::slash_command::tokenize(raw_text);
            let (positional, named) = split_args(tokens);
            Self {
                positional,
                named,
                raw: Some(raw),
            }
        } else {
            Self {
                positional: Vec::new(),
                named: cmd.args,
                raw: None,
            }
        }
    }
}

impl From<crate::ParsedCommand> for CommandArgs {
    fn from(cmd: crate::ParsedCommand) -> Self {
        let raw_text: String = cmd
            .raw
            .trim_start_matches('/')
            .split_once(char::is_whitespace)
            .map(|(_, rest)| rest.trim().to_string())
            .unwrap_or_default();
        let raw = if raw_text.is_empty() {
            None
        } else {
            Some(raw_text)
        };
        let (positional, named) = split_args(cmd.args);
        Self {
            positional,
            named,
            raw,
        }
    }
}

/// System or lifecycle event.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct EventPayload {
    /// Short string identifier for the event type.
    pub kind: String,
    /// Arbitrary structured payload for the event.
    pub data: serde_json::Value,
}

impl EventPayload {
    /// Create a new event payload.
    pub fn new(kind: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            kind: kind.into(),
            data,
        }
    }
}

/// Universal message envelope that flows through the entire system.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct Envelope {
    /// Unique message ID (UUID v7).
    pub id: MessageId,
    /// Source/destination channel identifier.
    pub channel: String,
    /// Session this message belongs to.
    pub session_id: SessionId,
    /// When the message was created.
    pub timestamp: DateTime<Utc>,
    /// Routing priority for the message queue.
    pub priority: Priority,
    /// The message content.
    pub payload: Payload,
    /// Adapter-specific and routing metadata.
    pub metadata: HashMap<String, serde_json::Value>,
    /// Distributed tracing propagation headers.
    pub trace_context: TraceContext,
    /// Canonical platform context produced by the adapter.
    ///
    /// Replaces the scattered platform-specific metadata keys
    /// (`telegram_chat_id`, `slack_channel`, etc.) with a typed, two-level
    /// model. Only the originating adapter writes `extensions`; shared code
    /// reads only the canonical fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_context: Option<PlatformContext>,
}

impl Envelope {
    /// Insert a metadata key-value pair.
    pub fn insert_meta(&mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) {
        self.metadata.insert(key.into(), value.into());
    }

    /// Create a text envelope with default priority and no metadata.
    pub fn text(
        channel: impl Into<String>,
        session_id: SessionId,
        text: impl Into<String>,
    ) -> Self {
        Self {
            id: MessageId::new(),
            channel: channel.into(),
            session_id,
            timestamp: Utc::now(),
            priority: Priority::default(),
            payload: Payload::Text(text.into()),
            metadata: HashMap::new(),
            trace_context: TraceContext::default(),
            platform_context: None,
        }
    }

    /// Create an envelope with an arbitrary payload, preserving priority and
    /// trace context from a source envelope.
    pub fn with_payload(
        channel: impl Into<String>,
        session_id: SessionId,
        payload: Payload,
        source: &Envelope,
    ) -> Self {
        Self {
            id: MessageId::new(),
            channel: channel.into(),
            session_id,
            timestamp: Utc::now(),
            priority: source.priority,
            payload,
            metadata: HashMap::new(),
            trace_context: source.trace_context.clone(),
            platform_context: source.platform_context.clone(),
        }
    }
}

/// Convert an [`InboundInteraction`] into an [`Envelope`] for the message bus.
///
/// This is the single conversion boundary between the public adapter contract
/// and the internal wire format. Called by the bridge task in `orka-server`.
impl From<InboundInteraction> for Envelope {
    fn from(interaction: InboundInteraction) -> Self {
        let payload = match interaction.content {
            InteractionContent::Text(text) => Payload::Text(text),
            InteractionContent::RichInput(RichInput { text, attachments }) => {
                Payload::RichInput(RichInputPayload {
                    text,
                    attachments: attachments
                        .into_iter()
                        .map(|a| MediaPayload {
                            mime_type: a.mime_type,
                            url: a.url,
                            filename: a.filename,
                            caption: a.caption,
                            size_bytes: a.size_bytes,
                            data_base64: a.data_base64,
                        })
                        .collect(),
                })
            }
            InteractionContent::Media(MediaAttachment {
                mime_type,
                url,
                filename,
                caption,
                size_bytes,
                data_base64,
            }) => Payload::Media(MediaPayload {
                mime_type,
                url,
                filename,
                caption,
                size_bytes,
                data_base64,
            }),
            InteractionContent::Command(CommandContent { name, args }) => {
                Payload::Command(CommandPayload { name, args })
            }
            InteractionContent::Event(EventContent { kind, data }) => {
                Payload::Event(EventPayload { kind, data })
            }
            // `InteractionContent` is `#[non_exhaustive]`; future variants fall
            // back to an empty text payload so the envelope is never silently
            // dropped.
            _ => Payload::Text(String::new()),
        };

        Self {
            id: MessageId::from(interaction.id),
            channel: interaction.source_channel,
            session_id: SessionId::from(interaction.session_id),
            timestamp: interaction.timestamp,
            priority: Priority::default(),
            payload,
            metadata: HashMap::new(),
            trace_context: interaction.trace,
            platform_context: Some(interaction.context),
        }
    }
}

/// Outbound message sent back to a channel.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct OutboundMessage {
    /// Destination channel to deliver the message to.
    pub channel: String,
    /// Session this reply belongs to.
    pub session_id: SessionId,
    /// The outbound message content.
    pub payload: Payload,
    /// Optional ID of the inbound message being replied to.
    pub reply_to: Option<MessageId>,
    /// Adapter-specific delivery metadata (legacy; prefer `platform_context`).
    pub metadata: HashMap<String, serde_json::Value>,
    /// Canonical platform context for routing.
    ///
    /// Set by the worker from the inbound envelope's `platform_context`.
    /// Adapters should read routing information from here first and fall back
    /// to `metadata` for backward compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_context: Option<PlatformContext>,
}

impl OutboundMessage {
    /// Create a new text outbound message.
    pub fn text(
        channel: impl Into<String>,
        session_id: SessionId,
        text: impl Into<String>,
        reply_to: Option<MessageId>,
    ) -> Self {
        Self {
            channel: channel.into(),
            session_id,
            payload: Payload::Text(text.into()),
            reply_to,
            metadata: HashMap::new(),
            platform_context: None,
        }
    }

    /// Create a new outbound message with the given payload.
    pub fn new(
        channel: impl Into<String>,
        session_id: SessionId,
        payload: Payload,
        reply_to: Option<MessageId>,
    ) -> Self {
        Self {
            channel: channel.into(),
            session_id,
            payload,
            reply_to,
            metadata: HashMap::new(),
            platform_context: None,
        }
    }

    /// Return the canonical chat / channel identifier from `platform_context`.
    ///
    /// This is the authoritative routing key for all adapters.  Every outbound
    /// message that travels through the standard worker→adapter pipeline will
    /// have `platform_context` populated from the originating inbound envelope,
    /// so adapters should call this instead of reading the legacy metadata bag.
    pub fn chat_id(&self) -> crate::Result<&str> {
        self.platform_context
            .as_ref()
            .and_then(|pc| pc.chat_id.as_deref())
            .ok_or_else(|| crate::Error::Other("missing platform_context.chat_id".into()))
    }

    /// Return a platform-specific extension value as `i64`.
    ///
    /// Extensions use the `{platform}_{field}` naming convention, e.g.
    /// `telegram_message_id`.  Returns `None` if the key is absent or is not
    /// an integer.
    pub fn extension_i64(&self, key: &str) -> Option<i64> {
        self.platform_context
            .as_ref()
            .and_then(|pc| pc.extensions.get(key))
            .and_then(serde_json::Value::as_i64)
    }

    /// Return a platform-specific extension value as `&str`.
    ///
    /// Returns `None` if the key is absent or is not a string.
    pub fn extension_str(&self, key: &str) -> Option<&str> {
        self.platform_context
            .as_ref()
            .and_then(|pc| pc.extensions.get(key))
            .and_then(serde_json::Value::as_str)
    }

    /// Set `source_channel` in metadata and return self (builder-style).
    #[must_use]
    pub fn with_source_channel(mut self, channel: &str) -> Self {
        self.metadata.insert(
            "source_channel".into(),
            serde_json::Value::String(channel.into()),
        );
        self
    }
}
