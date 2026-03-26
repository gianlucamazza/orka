//! Secret storage backends for Orka.
//!
//! Provides two implementations of [`orka_core::traits::SecretManager`]:
//!
//! - [`RedisSecretManager`]: Redis-backed, optional AES-256-GCM encryption.
//! - [`FileSecretManager`]: File-backed, no external infrastructure required.
//!   Suitable for local development and the `orka init` onboarding wizard.
//!
//! Also includes [`RotatingSecretManager`](rotation::RotatingSecretManager) for
//! zero-downtime secret rotation.

#![warn(missing_docs)]

/// Redis-backed secret manager with optional AES-256-GCM encryption.
pub mod redis_secret;

/// File-backed secret manager with optional AES-256-GCM encryption.
pub mod file_secret;

/// Secret rotation support for zero-downtime key rotation.
pub mod rotation;

use std::{path::PathBuf, sync::Arc};

use orka_core::{config::security::SecretBackend, traits::SecretManager};
use tracing::warn;

pub use crate::{
    file_secret::FileSecretManager,
    redis_secret::RedisSecretManager,
    rotation::{RotatingSecretManager, RotationConfig, RotationStatus},
};

/// Resolve the default file path for file-backed secret storage.
///
/// Uses `$XDG_CONFIG_HOME/orka/secrets.json` when the XDG variable is set,
/// otherwise falls back to `$HOME/.config/orka/secrets.json`.
pub fn default_secrets_file_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME").map_or_else(
        |_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config")
        },
        PathBuf::from,
    );
    base.join("orka").join("secrets.json")
}

fn resolve_encryption_key(config: &orka_core::config::OrkaConfig) -> Option<Vec<u8>> {
    config
        .secrets
        .encryption_key_env
        .as_deref()
        .or(Some("ORKA_SECRET_ENCRYPTION_KEY"))
        .and_then(|env_name| std::env::var(env_name).ok())
        .and_then(|hex_str| {
            let hex_str = hex_str.trim().to_string();
            if hex_str.is_empty() {
                return None;
            }
            match hex::decode(&hex_str) {
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
        })
}

/// Create a [`SecretManager`] from the given configuration.
///
/// Dispatches to the appropriate backend based on `config.secrets.backend`.
pub fn create_secret_manager(
    config: &orka_core::config::OrkaConfig,
) -> orka_core::Result<Arc<dyn SecretManager>> {
    let encryption_key = resolve_encryption_key(config);
    let key_ref = encryption_key.as_deref();

    if config.secrets.backend == SecretBackend::File {
        let path = config
            .secrets
            .file_path
            .as_deref()
            .map_or_else(default_secrets_file_path, PathBuf::from);
        return Ok(Arc::new(FileSecretManager::with_encryption(path, key_ref)?));
    }

    // Redis backend (default)
    if key_ref.is_none() {
        let env = std::env::var("ORKA_ENV")
            .or_else(|_| std::env::var("APP_ENV"))
            .unwrap_or_default();
        if env.eq_ignore_ascii_case("production") {
            return Err(orka_core::Error::secret(
                "ORKA_SECRET_ENCRYPTION_KEY must be set in production",
            ));
        }
        warn!(
            "ORKA_SECRET_ENCRYPTION_KEY not set — secrets stored in PLAINTEXT. \
             Do NOT use in production."
        );
        return Ok(Arc::new(RedisSecretManager::new(&config.redis.url)?));
    }

    Ok(Arc::new(RedisSecretManager::with_encryption(
        &config.redis.url,
        key_ref,
    )?))
}

/// Create a standalone file-backed [`SecretManager`] at the given path.
///
/// Optionally reads `ORKA_SECRET_ENCRYPTION_KEY` from the environment for
/// encryption at rest. Used by `orka init` before a full config is available.
pub fn create_file_secret_manager(
    path: impl Into<PathBuf>,
) -> orka_core::Result<Arc<dyn SecretManager>> {
    let encryption_key = std::env::var("ORKA_SECRET_ENCRYPTION_KEY")
        .ok()
        .and_then(|hex_str| {
            let hex_str = hex_str.trim().to_string();
            if hex_str.is_empty() {
                return None;
            }
            match hex::decode(&hex_str) {
                Ok(bytes) if bytes.len() == 32 => Some(bytes),
                _ => {
                    warn!("ORKA_SECRET_ENCRYPTION_KEY invalid or wrong length, ignoring");
                    None
                }
            }
        });
    let mgr = FileSecretManager::with_encryption(path.into(), encryption_key.as_deref())?;
    Ok(Arc::new(mgr))
}
