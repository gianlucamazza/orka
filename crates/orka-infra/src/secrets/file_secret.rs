//! File-backed secret manager with optional AES-256-GCM encryption.
//!
//! Secrets are persisted to a JSON file on disk. An in-process
//! [`tokio::sync::Mutex`] serialises concurrent access within the same
//! process; writes use an atomic rename to prevent partial-write corruption.

use std::{collections::HashMap, path::PathBuf, sync::Arc};
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt as _};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use orka_core::{Error, Result, SecretValue, traits::SecretManager};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use super::cipher::SecretCipher;

/// On-disk serialisation format for the secrets file.
#[derive(Debug, Serialize, Deserialize)]
struct SecretsFile {
    version: u32,
    /// Maps secret path → `base64(nonce || ciphertext)` (or base64(plaintext)
    /// in non-encrypted mode).
    #[serde(default)]
    secrets: HashMap<String, String>,
}

impl Default for SecretsFile {
    fn default() -> Self {
        Self {
            version: 1,
            secrets: HashMap::new(),
        }
    }
}

/// File-backed secret manager with optional AES-256-GCM encryption.
///
/// Unlike [`crate::secrets::RedisSecretManager`], this backend requires no
/// external infrastructure and is suitable for local development and the `orka
/// init` onboarding wizard.
pub struct FileSecretManager {
    path: PathBuf,
    cipher: Option<SecretCipher>,
    /// Serialises read-modify-write cycles within a single process.
    lock: Arc<Mutex<()>>,
}

impl FileSecretManager {
    /// Create a plaintext file-backed secret manager at `path`.
    ///
    /// Secrets are stored without encryption. Suitable for local development
    /// only.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            cipher: None,
            lock: Arc::new(Mutex::new(())),
        }
    }

    /// Create a file-backed secret manager with optional AES-256-GCM
    /// encryption.
    ///
    /// `encryption_key` must be exactly 32 bytes if `Some`.
    pub fn with_encryption(
        path: impl Into<PathBuf>,
        encryption_key: Option<&[u8]>,
    ) -> Result<Self> {
        let cipher = encryption_key.map(SecretCipher::new).transpose()?;
        Ok(Self {
            path: path.into(),
            cipher,
            lock: Arc::new(Mutex::new(())),
        })
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

    async fn read_file(&self) -> Result<SecretsFile> {
        match tokio::fs::read_to_string(&self.path).await {
            Ok(s) => serde_json::from_str(&s)
                .map_err(|e| Error::secret(format!("failed to parse secrets file: {e}"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SecretsFile::default()),
            Err(e) => Err(Error::secret(format!("failed to read secrets file: {e}"))),
        }
    }

    async fn write_file(&self, data: &SecretsFile) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::secret(format!("failed to create secrets directory: {e}")))?;
        }
        // Atomic write: write to a sibling temp file, then rename.
        let tmp = self.path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(data)
            .map_err(|e| Error::secret(format!("failed to serialise secrets: {e}")))?;
        tokio::fs::write(&tmp, &json)
            .await
            .map_err(|e| Error::secret(format!("failed to write temp secrets file: {e}")))?;
        tokio::fs::rename(&tmp, &self.path)
            .await
            .map_err(|e| Error::secret(format!("failed to rename secrets file: {e}")))?;
        #[cfg(unix)]
        tokio::fs::set_permissions(&self.path, Permissions::from_mode(0o600))
            .await
            .map_err(|e| Error::secret(format!("failed to set secrets file permissions: {e}")))?;
        Ok(())
    }
}

#[async_trait]
impl SecretManager for FileSecretManager {
    async fn get_secret(&self, path: &str) -> Result<SecretValue> {
        let _guard = self.lock.lock().await;
        let file = self.read_file().await?;
        let encoded = file
            .secrets
            .get(path)
            .ok_or_else(|| Error::secret(format!("not found: {path}")))?;
        let bytes = B64
            .decode(encoded)
            .map_err(|e| Error::secret(format!("base64 decode error: {e}")))?;
        let plaintext = self.decrypt(&bytes)?;
        debug!(path, "secret retrieved from file");
        Ok(SecretValue::new(plaintext))
    }

    async fn set_secret(&self, path: &str, value: &SecretValue) -> Result<()> {
        let _guard = self.lock.lock().await;
        let mut file = self.read_file().await?;
        let encrypted = self.encrypt(value.expose())?;
        file.secrets
            .insert(path.to_string(), B64.encode(&encrypted));
        self.write_file(&file).await?;
        debug!(path, "secret stored to file");
        Ok(())
    }

    async fn delete_secret(&self, path: &str) -> Result<()> {
        let _guard = self.lock.lock().await;
        let mut file = self.read_file().await?;
        file.secrets.remove(path);
        self.write_file(&file).await?;
        debug!(path, "secret deleted from file");
        Ok(())
    }

    async fn list_secrets(&self) -> Result<Vec<String>> {
        let _guard = self.lock.lock().await;
        let file = self.read_file().await?;
        let mut paths: Vec<String> = file.secrets.into_keys().collect();
        paths.sort();
        Ok(paths)
    }

    async fn migrate_plaintext_secrets(&self) -> Result<usize> {
        if self.cipher.is_none() {
            return Ok(0);
        }

        let _guard = self.lock.lock().await;
        let mut file = self.read_file().await?;
        let mut migrated = 0usize;

        for (path, encoded) in &mut file.secrets {
            let bytes = match B64.decode(encoded.as_str()) {
                Ok(b) => b,
                Err(e) => {
                    warn!(path, %e, "base64 decode failed, skipping migration");
                    continue;
                }
            };

            // Already encrypted — skip.
            if self.decrypt(&bytes).is_ok() {
                continue;
            }

            // Decryption failed. If the decoded bytes are valid UTF-8 this is
            // a plaintext secret that predates encryption being enabled.
            if std::str::from_utf8(&bytes).is_ok() {
                let encrypted = self.encrypt(&bytes)?;
                *encoded = B64.encode(&encrypted);
                info!(path, "migrated secret from plaintext to encrypted");
                migrated += 1;
            } else {
                warn!(
                    path,
                    "secret is neither valid ciphertext nor valid UTF-8, skipping migration"
                );
            }
        }

        if migrated > 0 {
            self.write_file(&file).await?;
        }

        Ok(migrated)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn tmp_path() -> PathBuf {
        tempfile::NamedTempFile::new()
            .expect("tempfile")
            .path()
            .with_extension("json")
    }

    fn mgr_plain(path: PathBuf) -> FileSecretManager {
        FileSecretManager::new(path)
    }

    fn mgr_encrypted(path: PathBuf) -> FileSecretManager {
        FileSecretManager::with_encryption(path, Some(&[0xABu8; 32])).unwrap()
    }

    #[test]
    fn invalid_key_length_rejected() {
        let result = FileSecretManager::with_encryption("/tmp/x.json", Some(&[0u8; 16]));
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn plaintext_roundtrip() {
        let p = tmp_path();
        let mgr = mgr_plain(p);
        let secret = SecretValue::new(b"hello world".to_vec());
        mgr.set_secret("test/key", &secret).await.unwrap();
        let got = mgr.get_secret("test/key").await.unwrap();
        assert_eq!(got.expose(), b"hello world");
    }

    #[tokio::test]
    async fn encrypted_roundtrip() {
        let p = tmp_path();
        let mgr = mgr_encrypted(p);
        let secret = SecretValue::new(b"s3cr3t!".to_vec());
        mgr.set_secret("llm/key", &secret).await.unwrap();
        let got = mgr.get_secret("llm/key").await.unwrap();
        assert_eq!(got.expose(), b"s3cr3t!");
    }

    #[tokio::test]
    async fn missing_secret_returns_error() {
        let p = tmp_path();
        let mgr = mgr_plain(p);
        assert!(mgr.get_secret("does/not/exist").await.is_err());
    }

    #[tokio::test]
    async fn delete_removes_secret() {
        let p = tmp_path();
        let mgr = mgr_plain(p);
        let secret = SecretValue::new(b"deleteme".to_vec());
        mgr.set_secret("to/delete", &secret).await.unwrap();
        mgr.delete_secret("to/delete").await.unwrap();
        assert!(mgr.get_secret("to/delete").await.is_err());
    }

    #[tokio::test]
    async fn list_returns_sorted_paths() {
        let p = tmp_path();
        let mgr = mgr_plain(p);
        mgr.set_secret("b/key", &SecretValue::new(b"1".to_vec()))
            .await
            .unwrap();
        mgr.set_secret("a/key", &SecretValue::new(b"2".to_vec()))
            .await
            .unwrap();
        let list = mgr.list_secrets().await.unwrap();
        assert_eq!(list, vec!["a/key", "b/key"]);
    }

    #[tokio::test]
    async fn persists_across_instances() {
        let p = tmp_path();
        {
            let mgr = mgr_encrypted(p.clone());
            mgr.set_secret("persist/test", &SecretValue::new(b"value".to_vec()))
                .await
                .unwrap();
        }
        // New instance, same path and same key
        let mgr2 = mgr_encrypted(p);
        let got = mgr2.get_secret("persist/test").await.unwrap();
        assert_eq!(got.expose(), b"value");
    }

    /// Secrets written by a plaintext manager can be read after migration.
    #[tokio::test]
    async fn migrate_plaintext_to_encrypted() {
        let p = tmp_path();

        // Write secrets with no encryption.
        let plain = mgr_plain(p.clone());
        plain
            .set_secret("token/a", &SecretValue::new(b"tok_aaa".to_vec()))
            .await
            .unwrap();
        plain
            .set_secret("token/b", &SecretValue::new(b"tok_bbb".to_vec()))
            .await
            .unwrap();

        // Re-open with encryption enabled and migrate.
        let enc = mgr_encrypted(p.clone());
        let migrated = enc.migrate_plaintext_secrets().await.unwrap();
        assert_eq!(migrated, 2);

        // All secrets should now be readable via the encrypted manager.
        assert_eq!(
            enc.get_secret("token/a").await.unwrap().expose(),
            b"tok_aaa"
        );
        assert_eq!(
            enc.get_secret("token/b").await.unwrap().expose(),
            b"tok_bbb"
        );
    }

    /// Migration is idempotent: running it twice migrates 0 on the second run.
    #[tokio::test]
    async fn migrate_is_idempotent() {
        let p = tmp_path();
        let plain = mgr_plain(p.clone());
        plain
            .set_secret("key", &SecretValue::new(b"value".to_vec()))
            .await
            .unwrap();

        let enc = mgr_encrypted(p);
        assert_eq!(enc.migrate_plaintext_secrets().await.unwrap(), 1);
        // Second call: already encrypted, nothing to migrate.
        assert_eq!(enc.migrate_plaintext_secrets().await.unwrap(), 0);
    }

    /// Migration skips secrets that are already encrypted.
    #[tokio::test]
    async fn migrate_skips_already_encrypted() {
        let p = tmp_path();
        // Write one secret with the encrypted manager (already encrypted).
        let enc = mgr_encrypted(p.clone());
        enc.set_secret("already/enc", &SecretValue::new(b"s3cr3t".to_vec()))
            .await
            .unwrap();

        // Migration should report 0 migrated.
        let migrated = enc.migrate_plaintext_secrets().await.unwrap();
        assert_eq!(migrated, 0);
        // Value still readable.
        assert_eq!(
            enc.get_secret("already/enc").await.unwrap().expose(),
            b"s3cr3t"
        );
    }

    /// Mixed state: some secrets already encrypted, some plaintext.
    /// Only plaintext ones should be migrated.
    #[tokio::test]
    async fn migrate_mixed_state() {
        let p = tmp_path();

        // Write one secret with no encryption (plaintext).
        let plain = mgr_plain(p.clone());
        plain
            .set_secret("plain/tok", &SecretValue::new(b"plain_value".to_vec()))
            .await
            .unwrap();

        // Write another secret directly with the encrypted manager.
        let enc = mgr_encrypted(p.clone());
        enc.set_secret("enc/tok", &SecretValue::new(b"enc_value".to_vec()))
            .await
            .unwrap();

        // Migrate: only the plaintext one should count.
        let migrated = enc.migrate_plaintext_secrets().await.unwrap();
        assert_eq!(migrated, 1);

        // Both secrets must be readable via the encrypted manager.
        assert_eq!(
            enc.get_secret("plain/tok").await.unwrap().expose(),
            b"plain_value"
        );
        assert_eq!(
            enc.get_secret("enc/tok").await.unwrap().expose(),
            b"enc_value"
        );
    }

    /// No encryption key → migration is a no-op returning 0.
    #[tokio::test]
    async fn migrate_no_op_without_encryption() {
        let p = tmp_path();
        let plain = mgr_plain(p.clone());
        plain
            .set_secret("k", &SecretValue::new(b"v".to_vec()))
            .await
            .unwrap();
        let migrated = plain.migrate_plaintext_secrets().await.unwrap();
        assert_eq!(migrated, 0);
    }
}
