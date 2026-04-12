use serde::Deserialize;

/// Secret storage backend selection.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum SecretBackend {
    /// Redis-backed secret storage (default).
    #[default]
    Redis,
    /// File-backed secret storage — no external infrastructure required.
    /// Suitable for local development and initial setup.
    File,
}

fn default_redis_url() -> String {
    "redis://127.0.0.1:6379".to_string()
}

/// Backward-compatible secret-specific Redis override fields.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct SecretRedisConfig {
    /// Redis connection URL (e.g. `"redis://127.0.0.1:6379"`).
    #[serde(default = "default_redis_url")]
    pub url: String,
}

impl Default for SecretRedisConfig {
    fn default() -> Self {
        Self {
            url: default_redis_url(),
        }
    }
}

/// Secret storage configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct SecretConfig {
    /// Secret storage backend.
    #[serde(default)]
    pub backend: SecretBackend,
    /// Path for file-backed storage (when `backend = "file"`).
    /// Defaults to `~/.config/orka/secrets.json`.
    pub file_path: Option<String>,
    /// Path to the master encryption key (hex-encoded, 32 bytes).
    pub encryption_key_path: Option<String>,
    /// Environment variable containing the encryption key.
    pub encryption_key_env: Option<String>,
    /// Legacy flattened Redis override fields retained for config
    /// compatibility.
    #[serde(flatten)]
    pub redis: SecretRedisConfig,
}

impl SecretConfig {
    /// Validate secret storage configuration.
    pub fn validate(&self) -> orka_core::Result<()> {
        Ok(())
    }
}
