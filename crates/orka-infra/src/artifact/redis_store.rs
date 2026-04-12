use async_trait::async_trait;
use deadpool_redis::Pool;
use orka_core::{
    ArtifactId, ConversationArtifact, ConversationId, Error, Result, traits::ArtifactStore,
};
use redis::AsyncCommands;
use uuid::Uuid;

/// TTL for orphaned artifacts (uploaded but not yet attached to a message): 24
/// hours.
const ORPHAN_ARTIFACT_TTL_SECS: u64 = 86_400;

/// Redis implementation of [`ArtifactStore`].
pub struct RedisArtifactStore {
    pool: Pool,
}

impl RedisArtifactStore {
    /// Connect to Redis and create a new artifact store.
    pub fn new(redis_url: &str) -> Result<Self> {
        let pool = crate::create_redis_pool(redis_url)
            .map_err(|e| Error::artifact(format!("failed to create Redis pool: {e}")))?;
        Ok(Self { pool })
    }

    fn metadata_key(id: &ArtifactId) -> String {
        format!("orka:artifact:{id}:meta")
    }

    fn bytes_key(id: &ArtifactId) -> String {
        format!("orka:artifact:{id}:bytes")
    }

    /// Secondary index: Redis SET of artifact ID strings keyed by conversation.
    fn conv_index_key(conv_id: &ConversationId) -> String {
        format!("orka:artifact:conv:{conv_id}")
    }
}

#[async_trait]
impl ArtifactStore for RedisArtifactStore {
    async fn put_artifact(&self, artifact: &ConversationArtifact, bytes: &[u8]) -> Result<()> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::artifact(format!("redis pool error: {e}")))?;

        let meta_key = Self::metadata_key(&artifact.id);
        let bytes_key = Self::bytes_key(&artifact.id);
        let metadata = serde_json::to_string(artifact)?;
        let id_str = artifact.id.to_string();

        if let Some(conv_id) = artifact.conversation_id {
            // Already attached to a conversation (e.g. AssistantOutput) — persist forever
            // and register in the per-conversation index.
            let conv_key = Self::conv_index_key(&conv_id);
            redis::pipe()
                .atomic()
                .set(&meta_key, &metadata)
                .ignore()
                .set(&bytes_key, bytes)
                .ignore()
                .sadd(&conv_key, &id_str)
                .ignore()
                .query_async::<()>(&mut conn)
                .await
                .map_err(|e| Error::artifact(format!("redis pipeline error: {e}")))?;
        } else {
            // Orphan upload — set TTL so unclaimed uploads do not accumulate forever.
            redis::pipe()
                .atomic()
                .set_ex(&meta_key, &metadata, ORPHAN_ARTIFACT_TTL_SECS)
                .ignore()
                .set_ex(&bytes_key, bytes, ORPHAN_ARTIFACT_TTL_SECS)
                .ignore()
                .query_async::<()>(&mut conn)
                .await
                .map_err(|e| Error::artifact(format!("redis pipeline error: {e}")))?;
        }
        Ok(())
    }

    async fn update_artifact(&self, artifact: &ConversationArtifact) -> Result<()> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::artifact(format!("redis pool error: {e}")))?;

        let meta_key = Self::metadata_key(&artifact.id);
        let bytes_key = Self::bytes_key(&artifact.id);
        let metadata = serde_json::to_string(artifact)?;

        if let Some(conv_id) = artifact.conversation_id {
            // Artifact is being attached to (or already lives in) a conversation:
            // persist both keys forever and register in the conversation index.
            let conv_key = Self::conv_index_key(&conv_id);
            let id_str = artifact.id.to_string();
            redis::pipe()
                .atomic()
                .set(&meta_key, &metadata)
                .ignore()
                .persist(&bytes_key)
                .ignore()
                .sadd(&conv_key, &id_str)
                .ignore()
                .query_async::<()>(&mut conn)
                .await
                .map_err(|e| Error::artifact(format!("redis pipeline error: {e}")))?;
        } else {
            // Still orphaned: overwrite metadata and refresh the TTL on both keys
            // so the expiry window resets from the time of the last update.
            redis::pipe()
                .atomic()
                .set_ex(&meta_key, &metadata, ORPHAN_ARTIFACT_TTL_SECS)
                .ignore()
                .expire(&bytes_key, ORPHAN_ARTIFACT_TTL_SECS as i64)
                .ignore()
                .query_async::<()>(&mut conn)
                .await
                .map_err(|e| Error::artifact(format!("redis pipeline error: {e}")))?;
        }
        Ok(())
    }

    async fn get_artifact(&self, artifact_id: &ArtifactId) -> Result<Option<ConversationArtifact>> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::artifact(format!("redis pool error: {e}")))?;
        let value: Option<String> = conn
            .get(Self::metadata_key(artifact_id))
            .await
            .map_err(|e| Error::artifact(format!("redis GET error: {e}")))?;
        value
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .map_err(Into::into)
    }

    async fn get_artifact_bytes(&self, artifact_id: &ArtifactId) -> Result<Option<Vec<u8>>> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::artifact(format!("redis pool error: {e}")))?;
        let value: Option<Vec<u8>> = conn
            .get(Self::bytes_key(artifact_id))
            .await
            .map_err(|e| Error::artifact(format!("redis GET error: {e}")))?;
        Ok(value)
    }

    async fn delete_artifact(&self, artifact_id: &ArtifactId) -> Result<()> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::artifact(format!("redis pool error: {e}")))?;

        let meta_key = Self::metadata_key(artifact_id);
        let bytes_key = Self::bytes_key(artifact_id);

        // Read metadata first so we can clean up the conversation index.
        let existing: Option<String> = conn
            .get(&meta_key)
            .await
            .map_err(|e| Error::artifact(format!("redis GET error: {e}")))?;

        let artifact: Option<ConversationArtifact> = existing
            .map(|json| serde_json::from_str::<ConversationArtifact>(&json))
            .transpose()?;

        // Delete both keys atomically.
        redis::pipe()
            .atomic()
            .del(&meta_key)
            .ignore()
            .del(&bytes_key)
            .ignore()
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| Error::artifact(format!("redis pipeline error: {e}")))?;

        // Remove from the per-conversation index if the artifact was attached.
        if let Some(artifact) = artifact
            && let Some(conv_id) = artifact.conversation_id
        {
            let conv_key = Self::conv_index_key(&conv_id);
            let _: i64 = conn
                .srem(conv_key, artifact_id.to_string())
                .await
                .map_err(|e| Error::artifact(format!("redis SREM error: {e}")))?;
        }
        Ok(())
    }

    async fn list_artifacts_by_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<ConversationArtifact>> {
        let mut conn = crate::retry::get_conn_with_retry(&self.pool)
            .await
            .map_err(|e| Error::artifact(format!("redis pool error: {e}")))?;

        let conv_key = Self::conv_index_key(conversation_id);
        let ids: Vec<String> = conn
            .smembers(&conv_key)
            .await
            .map_err(|e| Error::artifact(format!("redis SMEMBERS error: {e}")))?;

        let mut artifacts = Vec::with_capacity(ids.len());
        for id_str in ids {
            let artifact_id = Uuid::parse_str(&id_str)
                .map(ArtifactId::from)
                .map_err(|e| Error::artifact(format!("invalid artifact id in index: {e}")))?;
            let value: Option<String> = conn
                .get(Self::metadata_key(&artifact_id))
                .await
                .map_err(|e| Error::artifact(format!("redis GET error: {e}")))?;
            if let Some(json) = value {
                let artifact: ConversationArtifact = serde_json::from_str(&json)?;
                artifacts.push(artifact);
            }
        }
        artifacts.sort_by_key(|a| a.created_at);
        Ok(artifacts)
    }
}
