//! Domain service for conversation lifecycle operations.
//!
//! [`ConversationController`] encapsulates the business logic for cancelling
//! in-progress generations, retrying failed ones, deleting messages, and
//! marking conversations as read. It is free of HTTP concerns so it can be
//! used from any adapter (mobile REST, custom WebSocket, etc.).

use std::sync::Arc;

use chrono::Utc;

use crate::{
    ConversationId, ConversationMessage, ConversationMessageRole, ConversationStatus, Envelope,
    MessageId, SessionCancelTokens, SessionId,
    traits::{ConversationStore, MessageBus, MessageCursor},
    types::Conversation,
};

/// Errors returned by [`ConversationController`] methods.
#[derive(Debug)]
pub enum ControlError {
    /// The conversation or message was not found.
    NotFound,
    /// The caller does not own the conversation.
    NotOwned,
    /// The conversation is in the wrong state for the requested operation.
    InvalidState(&'static str),
    /// No active generation was found to cancel.
    NoActiveGeneration,
    /// An underlying store or bus operation failed.
    Store(crate::Error),
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "not found"),
            Self::NotOwned => write!(f, "not owned"),
            Self::InvalidState(msg) => write!(f, "invalid state: {msg}"),
            Self::NoActiveGeneration => write!(f, "no active generation to cancel"),
            Self::Store(e) => write!(f, "store error: {e}"),
        }
    }
}

impl std::error::Error for ControlError {}

impl From<crate::Error> for ControlError {
    fn from(e: crate::Error) -> Self {
        Self::Store(e)
    }
}

/// Result of a successful retry operation.
pub struct RetryResult {
    /// The user message that was re-published to the bus.
    pub message_id: MessageId,
    /// The session backing the conversation.
    pub session_id: SessionId,
    /// The conversation that was retried.
    pub conversation_id: ConversationId,
}

/// Domain service for conversation lifecycle operations.
///
/// Encapsulates cancel, retry, delete, and mark-read logic so that HTTP
/// handlers become thin wrappers (parse input → delegate → map error).
pub struct ConversationController {
    conversations: Arc<dyn ConversationStore>,
    bus: Arc<dyn MessageBus>,
    cancel_tokens: SessionCancelTokens,
}

impl ConversationController {
    /// Create a new controller.
    pub fn new(
        conversations: Arc<dyn ConversationStore>,
        bus: Arc<dyn MessageBus>,
        cancel_tokens: SessionCancelTokens,
    ) -> Self {
        Self { conversations, bus, cancel_tokens }
    }

    /// Load a conversation and verify that `user_id` is the owner.
    ///
    /// Returns [`ControlError::NotFound`] if the conversation does not exist
    /// *or* if the caller does not own it (to avoid leaking existence).
    pub async fn load_owned(
        &self,
        user_id: &str,
        id: ConversationId,
    ) -> std::result::Result<Conversation, ControlError> {
        let conversation = self
            .conversations
            .get_conversation(&id)
            .await
            .map_err(ControlError::Store)?
            .ok_or(ControlError::NotFound)?;

        if conversation.user_id != user_id {
            return Err(ControlError::NotFound);
        }

        Ok(conversation)
    }

    /// Cancel the active generation for a conversation.
    ///
    /// Signals the `CancellationToken` registered by the worker, resets the
    /// conversation status to `Active`, and persists the change.
    ///
    /// Returns [`ControlError::NoActiveGeneration`] if no token is registered.
    pub async fn cancel_generation(
        &self,
        conversation: &mut Conversation,
    ) -> std::result::Result<(), ControlError> {
        let cancelled = self
            .cancel_tokens
            .lock()
            .map(|tokens| {
                if let Some(token) = tokens.get(&conversation.session_id) {
                    token.cancel();
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false);

        if !cancelled {
            return Err(ControlError::NoActiveGeneration);
        }

        conversation.status = ConversationStatus::Active;
        conversation.updated_at = Utc::now();
        self.conversations
            .put_conversation(conversation)
            .await
            .map_err(ControlError::Store)
    }

    /// Retry the last failed generation for a conversation.
    ///
    /// Finds the last user message, deletes any trailing assistant messages,
    /// re-publishes the user message to the inbound bus, and resets the
    /// conversation status to `Active`.
    ///
    /// Returns [`ControlError::InvalidState`] if the conversation is not in
    /// `Failed` state, or [`ControlError::NotFound`] if no user message exists.
    pub async fn retry_generation(
        &self,
        conversation: &mut Conversation,
    ) -> std::result::Result<RetryResult, ControlError> {
        if conversation.status != ConversationStatus::Failed {
            return Err(ControlError::InvalidState("conversation is not in failed state"));
        }

        let messages = self
            .conversations
            .list_messages(&conversation.id, None, None, usize::MAX)
            .await
            .map_err(ControlError::Store)?;

        let user_message = find_last_user_message(&messages)
            .ok_or(ControlError::NotFound)?
            .clone();

        delete_trailing_messages(&self.conversations, &conversation.id, &messages, &user_message)
            .await?;

        let mut envelope =
            Envelope::text("mobile", conversation.session_id, &user_message.text);
        envelope.id = user_message.id;

        conversation.status = ConversationStatus::Active;
        conversation.updated_at = Utc::now();
        self.conversations
            .put_conversation(conversation)
            .await
            .map_err(ControlError::Store)?;

        self.bus
            .publish("inbound", &envelope)
            .await
            .map_err(ControlError::Store)?;

        Ok(RetryResult {
            message_id: user_message.id,
            session_id: conversation.session_id,
            conversation_id: conversation.id,
        })
    }

    /// Delete a single message from the conversation transcript.
    ///
    /// Returns [`ControlError::NotFound`] if the message does not exist.
    pub async fn delete_message(
        &self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> std::result::Result<(), ControlError> {
        let exists = self
            .conversations
            .get_message(conversation_id, message_id)
            .await
            .map_err(ControlError::Store)?
            .is_some();

        if !exists {
            return Err(ControlError::NotFound);
        }

        self.conversations
            .delete_message(conversation_id, message_id)
            .await
            .map_err(ControlError::Store)
    }

    /// Mark a message as read by setting a watermark for the user.
    ///
    /// Returns [`ControlError::NotFound`] if the message does not exist.
    pub async fn mark_read(
        &self,
        user_id: &str,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> std::result::Result<(), ControlError> {
        let message = self
            .conversations
            .get_message(conversation_id, message_id)
            .await
            .map_err(ControlError::Store)?
            .ok_or(ControlError::NotFound)?;

        let cursor = MessageCursor::from_message(&message);
        self.conversations
            .set_read_watermark(user_id, conversation_id, &cursor)
            .await
            .map_err(ControlError::Store)
    }
}

fn find_last_user_message(messages: &[ConversationMessage]) -> Option<&ConversationMessage> {
    messages.iter().rev().find(|m| m.role == ConversationMessageRole::User)
}

async fn delete_trailing_messages(
    store: &Arc<dyn ConversationStore>,
    conversation_id: &ConversationId,
    messages: &[ConversationMessage],
    user_message: &ConversationMessage,
) -> std::result::Result<(), ControlError> {
    let user_idx = messages
        .iter()
        .rposition(|m| m.id == user_message.id)
        .unwrap_or(messages.len());

    for msg in messages.iter().skip(user_idx + 1) {
        let _ = store.delete_message(conversation_id, &msg.id).await;
    }

    Ok(())
}

#[cfg(any(test, feature = "testing"))]
impl ConversationController {
    /// Create a controller backed by in-memory test doubles.
    pub fn new_in_memory(cancel_tokens: SessionCancelTokens) -> Self {
        use crate::testing::{InMemoryBus, InMemoryConversationStore};
        Self::new(
            Arc::new(InMemoryConversationStore::new()),
            Arc::new(InMemoryBus::new()),
            cancel_tokens,
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{
        ConversationMessage, ConversationMessageRole, ConversationStatus, MessageId, SessionId,
        testing::{InMemoryBus, InMemoryConversationStore},
        traits::ConversationStore,
        types::{Conversation, ConversationId},
    };

    fn make_cancel_tokens() -> SessionCancelTokens {
        Arc::new(Mutex::new(HashMap::new()))
    }

    fn make_controller(
        store: Arc<InMemoryConversationStore>,
        cancel_tokens: SessionCancelTokens,
    ) -> ConversationController {
        ConversationController::new(store, Arc::new(InMemoryBus::new()), cancel_tokens)
    }

    fn make_conversation(user_id: &str) -> Conversation {
        let id = ConversationId::new();
        Conversation::new(id, SessionId::from(id), user_id, "")
    }

    fn make_user_message(conversation_id: ConversationId) -> ConversationMessage {
        let conv = Conversation::new(
            conversation_id,
            SessionId::from(conversation_id),
            "alice",
            "",
        );
        ConversationMessage::new(
            MessageId::new(),
            conversation_id,
            conv.session_id,
            ConversationMessageRole::User,
            "hello",
        )
    }

    #[tokio::test]
    async fn load_owned_returns_conversation() {
        let store = Arc::new(InMemoryConversationStore::new());
        let conv = make_conversation("alice");
        store.put_conversation(&conv).await.unwrap();

        let ctrl = make_controller(store, make_cancel_tokens());
        let result = ctrl.load_owned("alice", conv.id).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn load_owned_wrong_user_returns_not_found() {
        let store = Arc::new(InMemoryConversationStore::new());
        let conv = make_conversation("alice");
        store.put_conversation(&conv).await.unwrap();

        let ctrl = make_controller(store, make_cancel_tokens());
        let result = ctrl.load_owned("bob", conv.id).await;
        assert!(matches!(result, Err(ControlError::NotFound)));
    }

    #[tokio::test]
    async fn cancel_generation_with_active_token() {
        let store = Arc::new(InMemoryConversationStore::new());
        let mut conv = make_conversation("alice");
        store.put_conversation(&conv).await.unwrap();

        let tokens = make_cancel_tokens();
        let token = CancellationToken::new();
        tokens.lock().unwrap().insert(conv.session_id, token.clone());

        let ctrl = make_controller(store, tokens);
        ctrl.cancel_generation(&mut conv).await.unwrap();

        assert!(token.is_cancelled());
        assert_eq!(conv.status, ConversationStatus::Active);
    }

    #[tokio::test]
    async fn cancel_generation_without_token_returns_error() {
        let store = Arc::new(InMemoryConversationStore::new());
        let mut conv = make_conversation("alice");
        store.put_conversation(&conv).await.unwrap();

        let ctrl = make_controller(store, make_cancel_tokens());
        let result = ctrl.cancel_generation(&mut conv).await;
        assert!(matches!(result, Err(ControlError::NoActiveGeneration)));
    }

    #[tokio::test]
    async fn retry_generation_from_failed_succeeds() {
        let store = Arc::new(InMemoryConversationStore::new());
        let mut conv = make_conversation("alice");
        conv.status = ConversationStatus::Failed;
        store.put_conversation(&conv).await.unwrap();

        let msg = make_user_message(conv.id);
        store.append_message(&msg).await.unwrap();

        let ctrl = make_controller(store, make_cancel_tokens());
        let result = ctrl.retry_generation(&mut conv).await;
        assert!(result.is_ok());
        assert_eq!(conv.status, ConversationStatus::Active);
    }

    #[tokio::test]
    async fn retry_generation_from_active_returns_invalid_state() {
        let store = Arc::new(InMemoryConversationStore::new());
        let mut conv = make_conversation("alice");
        store.put_conversation(&conv).await.unwrap();

        let ctrl = make_controller(store, make_cancel_tokens());
        let result = ctrl.retry_generation(&mut conv).await;
        assert!(matches!(result, Err(ControlError::InvalidState(_))));
    }

    #[tokio::test]
    async fn delete_message_existing() {
        let store = Arc::new(InMemoryConversationStore::new());
        let conv = make_conversation("alice");
        store.put_conversation(&conv).await.unwrap();
        let msg = make_user_message(conv.id);
        store.append_message(&msg).await.unwrap();

        let ctrl = make_controller(store, make_cancel_tokens());
        let result = ctrl.delete_message(&conv.id, &msg.id).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn delete_message_missing_returns_not_found() {
        let store = Arc::new(InMemoryConversationStore::new());
        let conv = make_conversation("alice");
        store.put_conversation(&conv).await.unwrap();

        let ctrl = make_controller(store, make_cancel_tokens());
        let result = ctrl.delete_message(&conv.id, &MessageId::new()).await;
        assert!(matches!(result, Err(ControlError::NotFound)));
    }

    #[tokio::test]
    async fn mark_read_existing_message() {
        let store = Arc::new(InMemoryConversationStore::new());
        let conv = make_conversation("alice");
        store.put_conversation(&conv).await.unwrap();
        let msg = make_user_message(conv.id);
        store.append_message(&msg).await.unwrap();

        let ctrl = make_controller(store, make_cancel_tokens());
        let result = ctrl.mark_read("alice", &conv.id, &msg.id).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn mark_read_missing_message_returns_not_found() {
        let store = Arc::new(InMemoryConversationStore::new());
        let conv = make_conversation("alice");
        store.put_conversation(&conv).await.unwrap();

        let ctrl = make_controller(store, make_cancel_tokens());
        let result = ctrl.mark_read("alice", &conv.id, &MessageId::new()).await;
        assert!(matches!(result, Err(ControlError::NotFound)));
    }
}
