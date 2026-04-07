//! Canonical platform context and sender identity types.
//!
//! Replaces the free-form `HashMap<String, Value>` metadata bag with a
//! two-level model:
//! - Canonical fields understood by orchestration and gateway
//! - An `extensions` bag for platform-specific overflow that only adapters read
//!   or write
//!
//! Neither struct is `#[non_exhaustive]` — adapters must be able to construct
//! them with struct literal syntax. New canonical fields are added as
//! `Option<_>` with `#[serde(default)]` so existing adapter code compiles
//! unchanged.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::TrustLevel;

/// Canonical routing and context information extracted from a platform message.
///
/// All fields that the gateway, worker, or orchestration layer need to read are
/// promoted to named fields. Anything platform-specific that no shared code
/// reads goes in `extensions`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PlatformContext {
    /// Identity of the sender.
    #[serde(default)]
    pub sender: SenderInfo,

    /// Primary chat or channel identifier (DM id, channel id, phone number).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,

    /// Thread or topic identifier within a channel, if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,

    /// Guild, server, or workspace identifier, if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,

    /// The message this interaction is a reply to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_target: Option<String>,

    /// Describes the interaction kind: `"direct"`, `"group"`, `"command"`,
    /// `"callback"`, etc. Used for priority routing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction_kind: Option<String>,

    /// Trust level asserted by the originating adapter.
    ///
    /// Set by adapters at message construction time. The gateway uses this to
    /// validate that messaging channels do not claim elevated trust they cannot
    /// prove (e.g. `UserAuthenticated` requires JWT evidence, not just a
    /// webhook signature).
    ///
    /// `None` means the adapter made no trust claim; the gateway accepts it
    /// without validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_level: Option<TrustLevel>,

    /// Platform-specific fields that no shared code needs to read.
    ///
    /// Keys follow `{platform}_{field}` convention, e.g.
    /// `telegram_message_thread_id`. Only the originating adapter should
    /// produce or consume these values.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    #[schema(value_type = Object, additional_properties = true)]
    pub extensions: HashMap<String, Value>,
}

/// Canonical identity of the sender of an inbound interaction.
#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SenderInfo {
    /// Platform-agnostic user identifier (e.g. Orka user ID, if known).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,

    /// Human-readable display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    /// The user's identifier on the originating platform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_user_id: Option<String>,
}
