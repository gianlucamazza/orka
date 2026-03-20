use async_trait::async_trait;
use deadpool_redis::{Config as DeadpoolConfig, Pool, Runtime};
use redis::AsyncCommands;
use tracing::debug;

use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};

use orka_core::traits::SecretManager;
use orka_core::{Error, Result, SecretValue};

/// AES-256-GCM nonce size in bytes.
const NONCE_SIZE: usize = 12;

/// Redis-backed secret manager with optional AES-256-GCM encryption.
pub struct RedisSecretManager {
    pool: Pool,
    /// AES-256-GCM cipher for encrypting secrets at rest.
    /// `None` means secrets are stored in plaintext (development mode).
    cipher: Option<Aes256Gcm>,
}

impl RedisSecretManager {
    /// Create a new secret manager.
    ///
    /// If `encryption_key` is `Some`, secrets are encrypted with AES-256-GCM
    /// before being written to Redis. The key must be exactly 32 bytes.
    /// If `None`, secrets are stored in plaintext (suitable for local development).
    pub fn new(redis_url: &str) -> Result<Self> {
        Self::with_encryption(redis_url, None)
    }

    /// Create a new secret manager with optional AES-256-GCM encryption.
    ///
    /// `encryption_key` must be exactly 32 bytes if provided. When `None`, secrets
    /// are stored in plaintext (suitable for local development only).
    pub fn with_encryption(redis_url: &str, encryption_key: Option<&[u8]>) -> Result<Self> {
        let cfg = DeadpoolConfig::from_url(redis_url);
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1))
            .map_err(|e| Error::secret(format!("failed to create Redis pool: {e}")))?;

        let cipher = if let Some(key) = encryption_key {
            if key.len() != 32 {
                return Err(Error::secret(
                    "encryption key must be exactly 32 bytes (AES-256)".to_string(),
                ));
            }
            Some(
                Aes256Gcm::new_from_slice(key)
                    .map_err(|e| Error::secret(format!("invalid encryption key: {e}")))?,
            )
        } else {
            None
        };

        Ok(Self { pool, cipher })
    }

    fn key(path: &str) -> String {
        format!("orka:secret:{path}")
    }

    /// Encrypt plaintext bytes. Returns nonce || ciphertext.
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let cipher = match &self.cipher {
            Some(c) => c,
            None => return Ok(plaintext.to_vec()),
        };
        let mut nonce_bytes = [0u8; NONCE_SIZE];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| Error::secret(format!("encryption failed: {e}")))?;

        let mut out = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    /// Decrypt nonce || ciphertext back to plaintext.
    fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        let cipher = match &self.cipher {
            Some(c) => c,
            None => return Ok(data.to_vec()),
        };
        if data.len() < NONCE_SIZE {
            return Err(Error::secret("encrypted data too short".to_string()));
        }
        let (nonce_bytes, ciphertext) = data.split_at(NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);

        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| Error::secret(format!("decryption failed: {e}")))
    }
}

#[async_trait]
impl SecretManager for RedisSecretManager {
    async fn get_secret(&self, path: &str) -> Result<SecretValue> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::secret(format!("redis pool error: {e}")))?;

        let value: Option<Vec<u8>> = conn
            .get(Self::key(path))
            .await
            .map_err(|e| Error::secret(format!("redis GET error: {e}")))?;

        match value {
            Some(bytes) => {
                let plaintext = self.decrypt(&bytes)?;
                debug!(path, "secret retrieved");
                Ok(SecretValue::new(plaintext))
            }
            None => Err(Error::secret(format!("not found: {path}"))),
        }
    }

    async fn set_secret(&self, path: &str, value: &SecretValue) -> Result<()> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::secret(format!("redis pool error: {e}")))?;

        let encrypted = self.encrypt(value.expose())?;

        let _: () = conn
            .set(Self::key(path), encrypted)
            .await
            .map_err(|e| Error::secret(format!("redis SET error: {e}")))?;

        debug!(path, "secret stored");
        Ok(())
    }

    async fn delete_secret(&self, path: &str) -> Result<()> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::secret(format!("redis pool error: {e}")))?;

        let _: () = conn
            .del(Self::key(path))
            .await
            .map_err(|e| Error::secret(format!("redis DEL error: {e}")))?;

        debug!(path, "secret deleted");
        Ok(())
    }

    async fn list_secrets(&self) -> Result<Vec<String>> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::secret(format!("redis pool error: {e}")))?;

        // Use SCAN instead of KEYS to avoid blocking Redis on large keyspaces
        let mut paths = Vec::new();
        let mut cursor: u64 = 0;
        loop {
            let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg("orka:secret:*")
                .arg("COUNT")
                .arg(100)
                .query_async(&mut *conn)
                .await
                .map_err(|e| Error::secret(format!("redis SCAN error: {e}")))?;

            for key in keys {
                if let Some(path) = key.strip_prefix("orka:secret:") {
                    paths.push(path.to_string());
                }
            }

            cursor = next_cursor;
            if cursor == 0 {
                break;
            }
        }

        Ok(paths)
    }
}
