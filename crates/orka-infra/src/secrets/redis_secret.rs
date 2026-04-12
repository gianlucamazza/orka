use async_trait::async_trait;
use deadpool_redis::Pool;
use orka_core::{Error, Result, SecretValue, traits::SecretManager};
use redis::AsyncCommands;
use tracing::{debug, info, warn};

use super::cipher::SecretCipher;

/// Redis-backed secret manager with optional AES-256-GCM encryption.
pub struct RedisSecretManager {
    pool: Pool,
    /// AES-256-GCM cipher for encrypting secrets at rest.
    /// `None` means secrets are stored in plaintext (development mode).
    cipher: Option<SecretCipher>,
}

impl RedisSecretManager {
    /// Create a new secret manager.
    ///
    /// If `encryption_key` is `Some`, secrets are encrypted with AES-256-GCM
    /// before being written to Redis. The key must be exactly 32 bytes.
    /// If `None`, secrets are stored in plaintext (suitable for local
    /// development).
    pub fn new(redis_url: &str) -> Result<Self> {
        Self::with_encryption(redis_url, None)
    }

    /// Create a new secret manager with optional AES-256-GCM encryption.
    ///
    /// `encryption_key` must be exactly 32 bytes if provided. When `None`,
    /// secrets are stored in plaintext (suitable for local development
    /// only).
    pub fn with_encryption(redis_url: &str, encryption_key: Option<&[u8]>) -> Result<Self> {
        let pool = crate::create_redis_pool(redis_url)
            .map_err(|e| Error::secret(format!("failed to create Redis pool: {e}")))?;
        let cipher = encryption_key.map(SecretCipher::new).transpose()?;
        Ok(Self { pool, cipher })
    }

    fn key(path: &str) -> String {
        format!("orka:secret:{path}")
    }

    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        match &self.cipher {
            Some(c) => c.encrypt(plaintext),
            None => Ok(plaintext.to_vec()),
        }
    }

    fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        match &self.cipher {
            Some(c) => c.decrypt(data),
            None => Ok(data.to_vec()),
        }
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

    async fn migrate_plaintext_secrets(&self) -> Result<usize> {
        if self.cipher.is_none() {
            return Ok(0);
        }

        let paths = self.list_secrets().await?;
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::secret(format!("redis pool error: {e}")))?;
        let mut migrated = 0usize;

        for path in &paths {
            let raw: Option<Vec<u8>> = conn
                .get(Self::key(path))
                .await
                .map_err(|e| Error::secret(format!("redis GET error: {e}")))?;
            let Some(raw) = raw else {
                continue;
            };

            // Already encrypted — skip.
            if self.decrypt(&raw).is_ok() {
                continue;
            }

            // Decryption failed. If the bytes are valid UTF-8 this is a
            // plaintext secret that predates encryption being enabled.
            if std::str::from_utf8(&raw).is_ok() {
                let encrypted = self.encrypt(&raw)?;
                let _: () = conn
                    .set(Self::key(path), encrypted)
                    .await
                    .map_err(|e| Error::secret(format!("redis SET error: {e}")))?;
                info!(path, "migrated secret from plaintext to encrypted");
                migrated += 1;
            } else {
                warn!(
                    path,
                    "secret is neither valid ciphertext nor valid UTF-8, skipping migration"
                );
            }
        }

        Ok(migrated)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Helper: create a manager with encryption, using a dummy Redis URL
    /// (pool creation succeeds but no actual connection is made until used).
    fn manager_with_key(key: &[u8; 32]) -> RedisSecretManager {
        RedisSecretManager::with_encryption("redis://localhost:6379", Some(key)).unwrap()
    }

    fn manager_plaintext() -> RedisSecretManager {
        RedisSecretManager::new("redis://localhost:6379").unwrap()
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = [0xABu8; 32];
        let mgr = manager_with_key(&key);
        let plaintext = b"super secret value";
        let encrypted = mgr.encrypt(plaintext).unwrap();
        assert_ne!(encrypted, plaintext); // must differ
        let decrypted = mgr.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn plaintext_mode_passthrough() {
        let mgr = manager_plaintext();
        let data = b"not encrypted";
        let encrypted = mgr.encrypt(data).unwrap();
        assert_eq!(encrypted, data); // no encryption
        let decrypted = mgr.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, data);
    }

    #[test]
    fn encrypt_produces_different_ciphertexts() {
        let key = [0x42u8; 32];
        let mgr = manager_with_key(&key);
        let plaintext = b"same input";
        let ct1 = mgr.encrypt(plaintext).unwrap();
        let ct2 = mgr.encrypt(plaintext).unwrap();
        // Different random nonces => different ciphertexts
        assert_ne!(ct1, ct2);
        // But both decrypt to the same plaintext
        assert_eq!(mgr.decrypt(&ct1).unwrap(), plaintext);
        assert_eq!(mgr.decrypt(&ct2).unwrap(), plaintext);
    }

    #[test]
    fn decrypt_too_short_data_fails() {
        let key = [0x01u8; 32];
        let mgr = manager_with_key(&key);
        // Less than NONCE_SIZE bytes
        let result = mgr.decrypt(&[0u8; 5]);
        assert!(result.is_err());
    }

    #[test]
    fn decrypt_tampered_ciphertext_fails() {
        let key = [0x99u8; 32];
        let mgr = manager_with_key(&key);
        let mut encrypted = mgr.encrypt(b"original").unwrap();
        // Flip a byte in the ciphertext portion
        let last = encrypted.len() - 1;
        encrypted[last] ^= 0xFF;
        assert!(mgr.decrypt(&encrypted).is_err());
    }

    #[test]
    fn invalid_key_length_fails() {
        let result =
            RedisSecretManager::with_encryption("redis://localhost:6379", Some(&[0u8; 16]));
        assert!(result.is_err());
    }

    #[test]
    fn key_prefix_format() {
        assert_eq!(RedisSecretManager::key("my/path"), "orka:secret:my/path");
    }

    #[test]
    fn encrypt_decrypt_empty_data() {
        let key = [0xCCu8; 32];
        let mgr = manager_with_key(&key);
        let encrypted = mgr.encrypt(b"").unwrap();
        let decrypted = mgr.decrypt(&encrypted).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn key_with_special_characters() {
        assert_eq!(
            RedisSecretManager::key("with spaces"),
            "orka:secret:with spaces"
        );
        assert_eq!(RedisSecretManager::key("a/b/c"), "orka:secret:a/b/c");
        assert_eq!(RedisSecretManager::key("ünïcödé"), "orka:secret:ünïcödé");
    }
}
