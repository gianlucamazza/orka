use async_trait::async_trait;
use chrono::Utc;
use deadpool_redis::{Config as DeadpoolConfig, Pool, Runtime};
use redis::AsyncCommands;
use tracing::debug;

use orka_core::traits::SessionStore;
use orka_core::{Error, Result, Session, SessionId};

pub struct RedisSessionStore {
    pool: Pool,
    ttl_secs: u64,
}

impl RedisSessionStore {
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
}
