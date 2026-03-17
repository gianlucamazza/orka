//! Encrypted secret storage backed by Redis.
//!
//! Provides [`RedisSecretManager`], an implementation of [`orka_core::traits::SecretManager`]
//! with optional AES-256-GCM encryption, and a [`create_secret_manager`] factory.

#![warn(missing_docs)]

#[allow(missing_docs)]
pub mod redis_secret;

pub use crate::redis_secret::RedisSecretManager;

use std::sync::Arc;

use orka_core::traits::SecretManager;
use tracing::warn;

/// Create a [`SecretManager`] from the given configuration.
pub fn create_secret_manager(
    config: &orka_core::config::OrkaConfig,
) -> orka_core::Result<Arc<dyn SecretManager>> {
    // Resolve encryption key from the configured env var (hex-encoded, 32 bytes = 64 hex chars)
    let encryption_key = config
        .secrets
        .encryption_key_env
        .as_deref()
        .or(Some("ORKA_SECRET_ENCRYPTION_KEY"))
        .and_then(|env_name| std::env::var(env_name).ok())
        .and_then(|hex_str| {
            let hex_str = hex_str.trim();
            if hex_str.is_empty() {
                return None;
            }
            match hex::decode(hex_str) {
                Ok(bytes) if bytes.len() == 32 => Some(bytes),
                Ok(bytes) => {
                    warn!(
                        len = bytes.len(),
                        "secret encryption key must be 32 bytes (64 hex chars), ignoring"
                    );
                    None
                }
                Err(e) => {
                    warn!(%e, "invalid hex in secret encryption key, ignoring");
                    None
                }
            }
        });

    let store = match &encryption_key {
        Some(key) => RedisSecretManager::with_encryption(&config.redis.url, Some(key))?,
        None => {
            let env = std::env::var("ORKA_ENV")
                .or_else(|_| std::env::var("APP_ENV"))
                .unwrap_or_default();
            if env.eq_ignore_ascii_case("production") {
                return Err(orka_core::Error::secret(
                    "ORKA_SECRET_ENCRYPTION_KEY must be set in production",
                ));
            }
            warn!(
                "ORKA_SECRET_ENCRYPTION_KEY not set — secrets stored in PLAINTEXT. Do NOT use in production."
            );
            RedisSecretManager::new(&config.redis.url)?
        }
    };

    Ok(Arc::new(store))
}
