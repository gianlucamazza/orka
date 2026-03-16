pub mod redis_secret;

pub use crate::redis_secret::RedisSecretManager;

use std::sync::Arc;

use orka_core::traits::SecretManager;
use tracing::{info, warn};

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
            info!("secret encryption key not configured — secrets stored in plaintext");
            RedisSecretManager::new(&config.redis.url)?
        }
    };

    Ok(Arc::new(store))
}
