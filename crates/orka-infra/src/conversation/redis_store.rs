use async_trait::async_trait;
use deadpool_redis::{Config as DeadpoolConfig, Pool, Runtime};
use orka_core::{
    Conversation, ConversationId, ConversationMessage, Error, Result, traits::ConversationStore,
};
use redis::AsyncCommands;

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
}

#[async_trait]
impl ConversationStore for RedisConversationStore {
    async fn put_conversation(&self, conversation: &Conversation) -> Result<()> {
        let mut conn = self
            .pool
            .get()
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
        let mut conn = self
            .pool
            .get()
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
    ) -> Result<Vec<Conversation>> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::bus(format!("redis pool error: {e}")))?;
        let start = offset as isize;
        let stop = offset.saturating_add(limit).saturating_sub(1) as isize;
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
            {
                conversations.push(conversation);
            }
        }

        Ok(conversations)
    }

    async fn append_message(&self, message: &ConversationMessage) -> Result<()> {
        self.upsert_message(message).await
    }

    async fn upsert_message(&self, message: &ConversationMessage) -> Result<()> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::bus(format!("redis pool error: {e}")))?;
        let key = Self::messages_key(&message.conversation_id);
        let values: Vec<String> = conn
            .lrange(&key, 0, -1)
            .await
            .map_err(|e| Error::bus(format!("redis LRANGE error: {e}")))?;
        let replacement = serde_json::to_string(message)?;
        if let Some((index, _)) = values
            .iter()
            .enumerate()
            .find(|(_, json)| serde_json::from_str::<ConversationMessage>(json).ok().is_some_and(|item| item.id == message.id))
        {
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
        let mut conn = self
            .pool
            .get()
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
        limit: Option<usize>,
        offset: usize,
    ) -> Result<Vec<ConversationMessage>> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::bus(format!("redis pool error: {e}")))?;
        let start = offset as isize;
        let stop = limit.map_or(-1, |value| {
            offset.saturating_add(value).saturating_sub(1) as isize
        });
        let values: Vec<String> = conn
            .lrange(Self::messages_key(conversation_id), start, stop)
            .await
            .map_err(|e| Error::bus(format!("redis LRANGE error: {e}")))?;

        let messages = values
            .into_iter()
            .filter_map(|json| serde_json::from_str::<ConversationMessage>(&json).ok())
            .collect::<Vec<_>>();

        Ok(messages)
    }
}
