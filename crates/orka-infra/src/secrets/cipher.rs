//! Shared AES-256-GCM cipher wrapper used by all secret manager backends.

use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, Generate, KeyInit},
    aes::cipher::consts::U12,
};
use orka_core::{Error, Result};

const NONCE_SIZE: usize = 12;
type GcmNonce = aes_gcm::Nonce<U12>;

/// Thin wrapper around [`Aes256Gcm`] for encrypt-then-prepend-nonce storage.
///
/// Both [`super::FileSecretManager`] and [`super::RedisSecretManager`] embed
/// an `Option<SecretCipher>`. When `None` the managers store values in
/// plaintext (development mode); when `Some` every value is encrypted before
/// persistence.
pub(crate) struct SecretCipher {
    inner: Aes256Gcm,
}

impl SecretCipher {
    /// Build a cipher from a 32-byte key.  Returns an error if the key length
    /// is not exactly 32 bytes.
    pub(super) fn new(key: &[u8]) -> Result<Self> {
        if key.len() != 32 {
            return Err(Error::secret(
                "encryption key must be exactly 32 bytes (AES-256)",
            ));
        }
        let inner = Aes256Gcm::new_from_slice(key)
            .map_err(|e| Error::secret(format!("invalid encryption key: {e}")))?;
        Ok(Self { inner })
    }

    /// Encrypt `plaintext` and return `nonce || ciphertext`.
    pub(super) fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let nonce = GcmNonce::generate();
        let ciphertext = self
            .inner
            .encrypt(&nonce, plaintext)
            .map_err(|e| Error::secret(format!("encryption failed: {e}")))?;
        let mut out = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        out.extend_from_slice(nonce.as_slice());
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    /// Decrypt `nonce || ciphertext` back to plaintext.
    pub(super) fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < NONCE_SIZE {
            return Err(Error::secret("encrypted data too short"));
        }
        let (nonce_bytes, ciphertext) = data.split_at(NONCE_SIZE);
        let nonce =
            GcmNonce::try_from(nonce_bytes).map_err(|_| Error::secret("invalid nonce length"))?;
        self.inner
            .decrypt(&nonce, ciphertext)
            .map_err(|e| Error::secret(format!("decryption failed: {e}")))
    }
}
