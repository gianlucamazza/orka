//! File-backed secret manager with optional AES-256-GCM encryption.
//!
//! Secrets are persisted to a JSON file on disk. An in-process
//! [`tokio::sync::Mutex`] serialises concurrent access within the same
//! process; writes use an atomic rename to prevent partial-write corruption.

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, OsRng, rand_core::RngCore},
};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use orka_core::{Error, Result, SecretValue, traits::SecretManager};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::debug;

/// AES-256-GCM nonce size in bytes.
const NONCE_SIZE: usize = 12;

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
/// Unlike [`crate::RedisSecretManager`], this backend requires no external
/// infrastructure and is suitable for local development and the `orka init`
/// onboarding wizard.
pub struct FileSecretManager {
    path: PathBuf,
    cipher: Option<Aes256Gcm>,
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
        let cipher = match encryption_key {
            Some(key) if key.len() == 32 => Some(
                Aes256Gcm::new_from_slice(key)
                    .map_err(|e| Error::secret(format!("invalid encryption key: {e}")))?,
            ),
            Some(_) => {
                return Err(Error::secret(
                    "encryption key must be exactly 32 bytes (AES-256)",
                ));
            }
            None => None,
        };
        Ok(Self {
            path: path.into(),
            cipher,
            lock: Arc::new(Mutex::new(())),
        })
    }

    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let Some(cipher) = &self.cipher else {
            return Ok(plaintext.to_vec());
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

    fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        let Some(cipher) = &self.cipher else {
            return Ok(data.to_vec());
        };
        if data.len() < NONCE_SIZE {
            return Err(Error::secret("encrypted data too short"));
        }
        let (nonce_bytes, ciphertext) = data.split_at(NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);
        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| Error::secret(format!("decryption failed: {e}")))
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
}

#[cfg(test)]
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
}
