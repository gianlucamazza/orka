use async_trait::async_trait;
use deadpool_redis::{Config as DeadpoolConfig, Pool, Runtime};
use orka_core::{
    Conversation, ConversationId, ConversationMessage, Error, Result,
    traits::{ConversationStore, MessageCursor, apply_message_cursors},
};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};

/// Redis implementation of [`ConversationStore`].
pub struct RedisConversationStore {
    pool: Pool,
}

impl RedisConversationStore {
    /// Connect to Redis and create a new conversation store.
    pub fn new(redis_url: &str) -> Result<Self> {
        let cfg = DeadpoolConfig::from_url(redis_url);
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1))
            .map_err(|e| Error::bus(format!("failed to create Redis pool: {e}")))?;
        Ok(Self { pool })
    }

    fn conversation_key(id: &ConversationId) -> String {
        format!("orka:conversation:{id}")
    }

    fn user_index_key(user_id: &str) -> String {
        format!("orka:user_conversations:{user_id}")
    }

    fn messages_key(id: &ConversationId) -> String {
        format!("orka:conversation_messages:{id}")
    }

    fn read_receipts_key(user_id: &str) -> String {
        format!("orka:read_receipts:{user_id}")
    }
}

#[derive(Serialize, Deserialize)]
struct WatermarkRecord {
    created_at_ms: i64,
    message_id: String,
}

#[async_trait]
impl ConversationStore for RedisConversationStore {
    async fn put_conversation(&self, conversation: &Conversation) -> Result<()> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::bus(format!("redis pool error: {e}")))?;

        let json = serde_json::to_string(conversation)?;
        let conversation_key = Self::conversation_key(&conversation.id);
        let user_index_key = Self::user_index_key(&conversation.user_id);
        let score = conversation.updated_at.timestamp_millis();

        let _: () = conn
            .set(&conversation_key, json)
            .await
            .map_err(|e| Error::bus(format!("redis SET error: {e}")))?;
        let _: () = conn
            .zadd(&user_index_key, conversation.id.to_string(), score)
            .await
            .map_err(|e| Error::bus(format!("redis ZADD error: {e}")))?;

        Ok(())
    }

    async fn get_conversation(&self, id: &ConversationId) -> Result<Option<Conversation>> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::bus(format!("redis pool error: {e}")))?;
        let value: Option<String> = conn
            .get(Self::conversation_key(id))
            .await
            .map_err(|e| Error::bus(format!("redis GET error: {e}")))?;
        value
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .map_err(Into::into)
    }

    async fn list_conversations(
        &self,
        user_id: &str,
        limit: usize,
        offset: usize,
        include_archived: bool,
    ) -> Result<Vec<Conversation>> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::bus(format!("redis pool error: {e}")))?;

        // Fetch enough IDs to satisfy the page after filtering. We overfetch
        // (3x) when excluding archived items to reduce round-trips.
        let fetch_limit = if include_archived {
            limit
        } else {
            limit.saturating_mul(3).max(limit)
        };
        let start = offset as isize;
        let stop = offset.saturating_add(fetch_limit).saturating_sub(1) as isize;
        let ids: Vec<String> = redis::cmd("ZREVRANGE")
            .arg(Self::user_index_key(user_id))
            .arg(start)
            .arg(stop)
            .query_async(&mut *conn)
            .await
            .map_err(|e| Error::bus(format!("redis ZREVRANGE error: {e}")))?;

        let mut conversations = Vec::with_capacity(ids.len());
        for id in ids {
            let value: Option<String> = conn
                .get(Self::conversation_key(&ConversationId::from(
                    uuid::Uuid::parse_str(&id).map_err(|e| {
                        Error::Other(format!("invalid conversation id in index: {e}"))
                    })?,
                )))
                .await
                .map_err(|e| Error::bus(format!("redis GET error: {e}")))?;
            if let Some(json) = value
                && let Ok(conversation) = serde_json::from_str::<Conversation>(&json)
                && (include_archived || conversation.archived_at.is_none())
            {
                conversations.push(conversation);
            }
            if conversations.len() >= limit {
                break;
            }
        }

        Ok(conversations)
    }

    async fn delete_conversation(&self, id: &ConversationId) -> Result<()> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::bus(format!("redis pool error: {e}")))?;

        // Retrieve the user_id needed to remove the sorted-set entry.
        let conversation_key = Self::conversation_key(id);
        let value: Option<String> = conn
            .get(&conversation_key)
            .await
            .map_err(|e| Error::bus(format!("redis GET error: {e}")))?;

        if let Some(json) = value {
            if let Ok(conversation) = serde_json::from_str::<Conversation>(&json) {
                let _: () = conn
                    .zrem(
                        Self::user_index_key(&conversation.user_id),
                        conversation.id.to_string(),
                    )
                    .await
                    .map_err(|e| Error::bus(format!("redis ZREM error: {e}")))?;
            }
            let _: () = conn
                .del(&conversation_key)
                .await
                .map_err(|e| Error::bus(format!("redis DEL error: {e}")))?;
            let _: () = conn
                .del(Self::messages_key(id))
                .await
                .map_err(|e| Error::bus(format!("redis DEL error: {e}")))?;
        }

        Ok(())
    }

    async fn append_message(&self, message: &ConversationMessage) -> Result<()> {
        self.upsert_message(message).await
    }

    async fn upsert_message(&self, message: &ConversationMessage) -> Result<()> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::bus(format!("redis pool error: {e}")))?;
        let key = Self::messages_key(&message.conversation_id);
        let values: Vec<String> = conn
            .lrange(&key, 0, -1)
            .await
            .map_err(|e| Error::bus(format!("redis LRANGE error: {e}")))?;
        let replacement = serde_json::to_string(message)?;
        if let Some((index, _)) = values.iter().enumerate().find(|(_, json)| {
            serde_json::from_str::<ConversationMessage>(json)
                .ok()
                .is_some_and(|item| item.id == message.id)
        }) {
            let _: () = conn
                .lset(&key, index as isize, replacement)
                .await
                .map_err(|e| Error::bus(format!("redis LSET error: {e}")))?;
        } else {
            let _: () = conn
                .rpush(&key, replacement)
                .await
                .map_err(|e| Error::bus(format!("redis RPUSH error: {e}")))?;
        }
        Ok(())
    }

    async fn get_message(
        &self,
        conversation_id: &ConversationId,
        message_id: &orka_core::MessageId,
    ) -> Result<Option<ConversationMessage>> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::bus(format!("redis pool error: {e}")))?;
        let values: Vec<String> = conn
            .lrange(Self::messages_key(conversation_id), 0, -1)
            .await
            .map_err(|e| Error::bus(format!("redis LRANGE error: {e}")))?;

        Ok(values.into_iter().find_map(|json| {
            serde_json::from_str::<ConversationMessage>(&json)
                .ok()
                .filter(|item| &item.id == message_id)
        }))
    }

    async fn list_messages(
        &self,
        conversation_id: &ConversationId,
        after: Option<&MessageCursor>,
        before: Option<&MessageCursor>,
        limit: usize,
    ) -> Result<Vec<ConversationMessage>> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::bus(format!("redis pool error: {e}")))?;
        let values: Vec<String> = conn
            .lrange(Self::messages_key(conversation_id), 0, -1)
            .await
            .map_err(|e| Error::bus(format!("redis LRANGE error: {e}")))?;

        let mut all = values
            .into_iter()
            .filter_map(|json| serde_json::from_str::<ConversationMessage>(&json).ok())
            .collect::<Vec<_>>();
        all.sort_by_key(|m| (m.created_at, m.id.as_uuid()));

        Ok(apply_message_cursors(all, after, before, limit))
    }

    async fn set_read_watermark(
        &self,
        user_id: &str,
        conversation_id: &ConversationId,
        cursor: &MessageCursor,
    ) -> Result<()> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::bus(format!("redis pool error: {e}")))?;
        let key = Self::read_receipts_key(user_id);
        let field = conversation_id.to_string();

        // Only advance — never move backward.
        let existing: Option<String> = conn
            .hget(&key, &field)
            .await
            .map_err(|e| Error::bus(format!("redis HGET error: {e}")))?;
        let should_update = existing.map_or(true, |json| {
            serde_json::from_str::<WatermarkRecord>(&json).map_or(true, |rec| {
                cursor.created_at_ms > rec.created_at_ms
                    || (cursor.created_at_ms == rec.created_at_ms
                        && cursor.message_id.to_string() > rec.message_id)
            })
        });
        if should_update {
            let record = WatermarkRecord {
                created_at_ms: cursor.created_at_ms,
                message_id: cursor.message_id.to_string(),
            };
            let json = serde_json::to_string(&record)?;
            let _: () = conn
                .hset(&key, &field, json)
                .await
                .map_err(|e| Error::bus(format!("redis HSET error: {e}")))?;
        }
        Ok(())
    }

    async fn get_read_watermark(
        &self,
        user_id: &str,
        conversation_id: &ConversationId,
    ) -> Result<Option<MessageCursor>> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::bus(format!("redis pool error: {e}")))?;
        let value: Option<String> = conn
            .hget(
                Self::read_receipts_key(user_id),
                conversation_id.to_string(),
            )
            .await
            .map_err(|e| Error::bus(format!("redis HGET error: {e}")))?;
        let Some(json) = value else {
            return Ok(None);
        };
        let record: WatermarkRecord = serde_json::from_str(&json)?;
        let message_id = uuid::Uuid::parse_str(&record.message_id)
            .map_err(|e| Error::Other(format!("invalid watermark uuid: {e}")))?;
        Ok(Some(MessageCursor {
            created_at_ms: record.created_at_ms,
            message_id,
        }))
    }
}
