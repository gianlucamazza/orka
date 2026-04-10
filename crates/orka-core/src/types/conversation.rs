//! Conversation, message, and artifact types for the product-facing mobile API.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::ids::{ArtifactId, ConversationId, MessageId, SessionId};

/// Lifecycle state for a product-facing conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ConversationStatus {
    /// Conversation is active and can accept more messages.
    #[default]
    Active,
    /// Conversation is paused waiting for a human or external decision.
    Interrupted,
    /// Conversation completed with an error state.
    Failed,
}

/// Product-facing conversation metadata used by mobile clients.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct Conversation {
    /// Stable conversation identifier.
    pub id: ConversationId,
    /// Runtime session backing this conversation.
    pub session_id: SessionId,
    /// Owning authenticated user.
    pub user_id: String,
    /// Human-readable conversation title.
    pub title: String,
    /// Preview of the most recent user-facing message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_preview: Option<String>,
    /// Lifecycle state shown to clients.
    pub status: ConversationStatus,
    /// When this conversation was archived, or `None` if active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    /// Whether the conversation is pinned by the user.
    #[serde(default)]
    pub pinned: bool,
    /// User-defined labels attached to this conversation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Workspace this conversation is bound to, or `None` for the server
    /// default workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Conversation creation time.
    pub created_at: DateTime<Utc>,
    /// Last update time.
    pub updated_at: DateTime<Utc>,
}

impl Conversation {
    /// Create a new active conversation.
    pub fn new(
        id: ConversationId,
        session_id: SessionId,
        user_id: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id,
            session_id,
            user_id: user_id.into(),
            title: title.into(),
            last_message_preview: None,
            status: ConversationStatus::Active,
            archived_at: None,
            pinned: false,
            tags: Vec::new(),
            workspace: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Bind this conversation to a named workspace.
    #[must_use]
    pub fn with_workspace(mut self, workspace: impl Into<String>) -> Self {
        self.workspace = Some(workspace.into());
        self
    }
}

/// Source of a conversation artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConversationArtifactOrigin {
    /// Uploaded by the end user from the client device.
    UserUpload,
    /// Produced by Orka during assistant execution.
    AssistantOutput,
}

/// Product-facing artifact metadata associated with a conversation.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct ConversationArtifact {
    /// Stable artifact identifier.
    pub id: ArtifactId,
    /// Owning authenticated user.
    pub owner_user_id: String,
    /// Owning conversation, if already attached to one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<ConversationId>,
    /// Owning message, if already attached to one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<MessageId>,
    /// Where the artifact originated.
    pub origin: ConversationArtifactOrigin,
    /// MIME type of the content.
    pub mime_type: String,
    /// Suggested filename for downloads and previews.
    pub filename: String,
    /// Optional caption or user-provided note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    /// Byte size, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    /// Width in pixels for visual media, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Height in pixels for visual media, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Duration in milliseconds for timed media, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Artifact creation time.
    pub created_at: DateTime<Utc>,
}

impl ConversationArtifact {
    /// Create a new conversation artifact metadata record.
    pub fn new(
        owner_user_id: impl Into<String>,
        origin: ConversationArtifactOrigin,
        mime_type: impl Into<String>,
        filename: impl Into<String>,
    ) -> Self {
        Self {
            id: ArtifactId::new(),
            owner_user_id: owner_user_id.into(),
            conversation_id: None,
            message_id: None,
            origin,
            mime_type: mime_type.into(),
            filename: filename.into(),
            caption: None,
            size_bytes: None,
            width: None,
            height: None,
            duration_ms: None,
            created_at: Utc::now(),
        }
    }
}

/// Role of a transcript message in a product-facing conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ConversationMessageRole {
    /// End-user authored message.
    User,
    /// Assistant authored message.
    Assistant,
}

/// Delivery state of a transcript message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ConversationMessageStatus {
    /// Message has been accepted but not finalized yet.
    Pending,
    /// Message has been fully committed.
    Completed,
    /// Message generation failed.
    Failed,
}

/// Product-facing transcript message for mobile clients.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct ConversationMessage {
    /// Stable message identifier.
    pub id: MessageId,
    /// Owning conversation.
    pub conversation_id: ConversationId,
    /// Runtime session backing this message.
    pub session_id: SessionId,
    /// User-visible role.
    pub role: ConversationMessageRole,
    /// Message text content.
    pub text: String,
    /// Associated artifacts for this message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ConversationArtifact>,
    /// Persistence / delivery state.
    pub status: ConversationMessageStatus,
    /// Message creation time.
    pub created_at: DateTime<Utc>,
    /// Finalization time when completed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalized_at: Option<DateTime<Utc>>,
}

impl ConversationMessage {
    /// Create a completed user-facing message.
    pub fn new(
        id: MessageId,
        conversation_id: ConversationId,
        session_id: SessionId,
        role: ConversationMessageRole,
        text: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id,
            conversation_id,
            session_id,
            role,
            text: text.into(),
            artifacts: Vec::new(),
            status: ConversationMessageStatus::Completed,
            created_at: now,
            finalized_at: Some(now),
        }
    }
}
