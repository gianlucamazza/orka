use async_trait::async_trait;
use chrono::Utc;
use deadpool_redis::{Config as DeadpoolConfig, Pool, Runtime};
use orka_core::{Error, Result, Session, SessionId, traits::SessionStore};
use redis::AsyncCommands;
use tracing::debug;

/// Redis implementation of [`orka_core::traits::SessionStore`].
pub struct RedisSessionStore {
    pool: Pool,
    ttl_secs: u64,
}

impl RedisSessionStore {
    /// Connect to Redis and create a new session store with the given TTL.
    pub fn new(redis_url: &str, ttl_secs: u64) -> Result<Self> {
        let cfg = DeadpoolConfig::from_url(redis_url);
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1))
            .map_err(|e| Error::bus(format!("failed to create Redis pool: {e}")))?;

        Ok(Self { pool, ttl_secs })
    }

    fn key(id: &SessionId) -> String {
        format!("orka:session:{id}")
    }
}

#[async_trait]
impl SessionStore for RedisSessionStore {
    async fn get(&self, id: &SessionId) -> Result<Option<Session>> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::bus(format!("redis pool error: {e}")))?;

        let value: Option<String> = conn
            .get(Self::key(id))
            .await
            .map_err(|e| Error::bus(format!("redis GET error: {e}")))?;

        match value {
            Some(json) => {
                let session: Session = serde_json::from_str(&json)?;
                debug!(session_id = %id, "session retrieved");
                Ok(Some(session))
            }
            None => Ok(None),
        }
    }

    async fn put(&self, session: &Session) -> Result<()> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::bus(format!("redis pool error: {e}")))?;

        let mut session = session.clone();
        session.updated_at = Utc::now();

        let json = serde_json::to_string(&session)?;
        let _: () = conn
            .set_ex(Self::key(&session.id), json, self.ttl_secs)
            .await
            .map_err(|e| Error::bus(format!("redis SET error: {e}")))?;

        debug!(session_id = %session.id, "session stored");
        Ok(())
    }

    async fn delete(&self, id: &SessionId) -> Result<()> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::bus(format!("redis pool error: {e}")))?;

        let _: () = conn
            .del(Self::key(id))
            .await
            .map_err(|e| Error::bus(format!("redis DEL error: {e}")))?;

        debug!(session_id = %id, "session deleted");
        Ok(())
    }

    async fn list(&self, limit: usize) -> Result<Vec<Session>> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::bus(format!("redis pool error: {e}")))?;

        let keys: Vec<String> = redis::cmd("SCAN")
            .arg(0i64)
            .arg("MATCH")
            .arg("orka:session:*")
            .arg("COUNT")
            .arg(limit.max(100) as i64)
            .query_async(&mut *conn)
            .await
            .map(|(_cursor, keys): (i64, Vec<String>)| keys)
            .unwrap_or_default();

        let mut sessions = Vec::new();
        for key in keys.iter().take(limit) {
            let value: Option<String> = conn
                .get(key)
                .await
                .map_err(|e| Error::bus(format!("redis GET error: {e}")))?;
            if let Some(json) = value
                && let Ok(session) = serde_json::from_str::<Session>(&json)
            {
                sessions.push(session);
            }
        }
        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(sessions)
    }
}
