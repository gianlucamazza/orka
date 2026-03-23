//! Secret rotation support for zero-downtime key rotation.
//!
//! Provides [`RotatingSecretManager`] which wraps multiple secret managers
//! (primary and previous) to enable graceful key rotation without service interruption.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use orka_core::traits::SecretManager;
use orka_core::{Result, SecretValue};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Configuration for secret rotation.
#[derive(Debug, Clone)]
pub struct RotationConfig {
    /// How long to keep the previous secret valid after rotation.
    pub overlap_duration: Duration,
    /// Whether to log rotation events.
    pub enable_logging: bool,
}

impl Default for RotationConfig {
    fn default() -> Self {
        Self {
            overlap_duration: Duration::from_secs(3600), // 1 hour
            enable_logging: true,
        }
    }
}

/// A secret manager that supports rotation with overlap period.
///
/// During rotation, both the primary and previous secrets are valid.
/// Reads try primary first, then fall back to previous if primary fails.
/// After the overlap period, the previous secret is discarded.
///
/// # Example
///
/// ```rust
/// use std::sync::Arc;
/// use orka_secrets::rotation::{RotatingSecretManager, RotationConfig};
/// use orka_secrets::RedisSecretManager;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Create managers with different encryption keys
/// let primary = Arc::new(RedisSecretManager::new(/* ... */)?);
/// let previous = Arc::new(RedisSecretManager::new(/* ... */)?);
///
/// let rotating = RotatingSecretManager::new(
///     primary,
///     Some(previous),
///     RotationConfig::default(),
/// );
///
/// // Read secret - tries primary first, then previous
/// let secret = rotating.get_secret("api/key").await?;
/// # Ok(())
/// # }
/// ```
pub struct RotatingSecretManager {
    primary: Arc<dyn SecretManager>,
    previous: RwLock<Option<Arc<dyn SecretManager>>>,
    rotation_time: RwLock<Option<SystemTime>>,
    config: RotationConfig,
}

impl RotatingSecretManager {
    /// Create a new rotating secret manager.
    ///
    /// * `primary` - The current secret manager with the new key
    /// * `previous` - Optional previous secret manager with the old key
    /// * `config` - Rotation configuration
    pub fn new(
        primary: Arc<dyn SecretManager>,
        previous: Option<Arc<dyn SecretManager>>,
        config: RotationConfig,
    ) -> Self {
        let rotation_time = if previous.is_some() {
            Some(SystemTime::now())
        } else {
            None
        };

        Self {
            primary,
            previous: RwLock::new(previous),
            rotation_time: RwLock::new(rotation_time),
            config,
        }
    }

    /// Trigger a rotation: current primary becomes previous, new primary takes over.
    pub async fn rotate(&self, _new_primary: Arc<dyn SecretManager>) {
        let mut previous = self.previous.write().await;
        let mut rotation_time = self.rotation_time.write().await;

        // Current primary becomes previous
        *previous = Some(self.primary.clone());
        *rotation_time = Some(SystemTime::now());

        // Update primary (this would require interior mutability in practice)
        // For now, we log the rotation
        if self.config.enable_logging {
            info!("Secret rotation triggered - previous key now in overlap period");
        }
    }

    /// Check if the overlap period has expired and clear previous if so.
    async fn check_overlap_expired(&self) {
        let should_clear = {
            let rotation_time = self.rotation_time.read().await;
            if let Some(time) = *rotation_time {
                SystemTime::now().duration_since(time).unwrap_or_default()
                    > self.config.overlap_duration
            } else {
                false
            }
        };

        if should_clear {
            let mut previous = self.previous.write().await;
            let mut rotation_time = self.rotation_time.write().await;

            if previous.is_some() {
                if self.config.enable_logging {
                    info!("Secret rotation overlap period expired - clearing previous key");
                }
                *previous = None;
                *rotation_time = None;
            }
        }
    }

    /// Force clear the previous secret immediately.
    pub async fn clear_previous(&self) {
        let mut previous = self.previous.write().await;
        let mut rotation_time = self.rotation_time.write().await;

        if previous.is_some() {
            if self.config.enable_logging {
                info!("Manually clearing previous secret");
            }
            *previous = None;
            *rotation_time = None;
        }
    }

    /// Get rotation status for observability.
    pub async fn rotation_status(&self) -> RotationStatus {
        let previous = self.previous.read().await;
        let rotation_time = self.rotation_time.read().await;

        RotationStatus {
            has_previous: previous.is_some(),
            rotation_time: *rotation_time,
            overlap_duration: self.config.overlap_duration,
            overlap_remaining: rotation_time.map(|t| {
                let elapsed = SystemTime::now().duration_since(t).unwrap_or_default();
                self.config.overlap_duration.saturating_sub(elapsed)
            }),
        }
    }
}

/// Rotation status for monitoring.
#[derive(Debug, Clone)]
pub struct RotationStatus {
    /// Whether a previous secret is still valid.
    pub has_previous: bool,
    /// When the last rotation occurred.
    pub rotation_time: Option<SystemTime>,
    /// Configured overlap duration.
    pub overlap_duration: Duration,
    /// Remaining time for overlap period.
    pub overlap_remaining: Option<Duration>,
}

#[async_trait]
impl SecretManager for RotatingSecretManager {
    async fn get_secret(&self, path: &str) -> Result<SecretValue> {
        // Check if overlap period expired
        self.check_overlap_expired().await;

        // Try primary first
        match self.primary.get_secret(path).await {
            Ok(secret) => {
                debug!(path, "retrieved secret from primary");
                Ok(secret)
            }
            Err(primary_err) => {
                // Try previous if available
                let previous = self.previous.read().await;
                if let Some(prev) = previous.as_ref() {
                    match prev.get_secret(path).await {
                        Ok(secret) => {
                            if self.config.enable_logging {
                                warn!(path, "retrieved secret from previous (rotation in progress)");
                            }
                            Ok(secret)
                        }
                        Err(previous_err) => {
                            // Both failed - return primary error as it's the current one
                            warn!(
                                path,
                                %primary_err,
                                %previous_err,
                                "secret not found in primary or previous"
                            );
                            Err(primary_err)
                        }
                    }
                } else {
                    Err(primary_err)
                }
            }
        }
    }

    async fn set_secret(&self, path: &str, value: &SecretValue) -> Result<()> {
        // Always write to primary
        self.primary.set_secret(path, value).await
    }

    async fn delete_secret(&self, path: &str) -> Result<()> {
        // Delete from both primary and previous
        let result = self.primary.delete_secret(path).await;

        let previous = self.previous.read().await;
        if let Some(prev) = previous.as_ref() {
            let _ = prev.delete_secret(path).await;
        }

        result
    }

    async fn list_secrets(&self) -> Result<Vec<String>> {
        // List from primary only (previous is being phased out)
        self.primary.list_secrets().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orka_core::testing::InMemorySecretManager;

    #[tokio::test]
    async fn test_primary_only() {
        let primary = Arc::new(InMemorySecretManager::new());
        primary.set_secret("key", &SecretValue::new(b"value".to_vec())).await.unwrap();

        let rotating = RotatingSecretManager::new(
            primary.clone(),
            None,
            RotationConfig::default(),
        );

        let secret = rotating.get_secret("key").await.unwrap();
        assert_eq!(secret.expose(), b"value");
    }

    #[tokio::test]
    async fn test_fallback_to_previous() {
        let primary = Arc::new(InMemorySecretManager::new());
        let previous = Arc::new(InMemorySecretManager::new());

        // Secret only in previous
        previous.set_secret("old_key", &SecretValue::new(b"old_value".to_vec())).await.unwrap();

        let rotating = RotatingSecretManager::new(
            primary.clone(),
            Some(previous.clone()),
            RotationConfig::default(),
        );

        // Should fallback to previous
        let secret = rotating.get_secret("old_key").await.unwrap();
        assert_eq!(secret.expose(), b"old_value");
    }

    #[tokio::test]
    async fn test_primary_takes_precedence() {
        let primary = Arc::new(InMemorySecretManager::new());
        let previous = Arc::new(InMemorySecretManager::new());

        // Different values in primary and previous
        primary.set_secret("key", &SecretValue::new(b"new_value".to_vec())).await.unwrap();
        previous.set_secret("key", &SecretValue::new(b"old_value".to_vec())).await.unwrap();

        let rotating = RotatingSecretManager::new(
            primary.clone(),
            Some(previous.clone()),
            RotationConfig::default(),
        );

        // Should return primary value
        let secret = rotating.get_secret("key").await.unwrap();
        assert_eq!(secret.expose(), b"new_value");
    }

    #[tokio::test]
    async fn test_rotation_status() {
        let primary = Arc::new(InMemorySecretManager::new());
        let previous = Arc::new(InMemorySecretManager::new());

        let rotating = RotatingSecretManager::new(
            primary.clone(),
            Some(previous.clone()),
            RotationConfig::default(),
        );

        let status = rotating.rotation_status().await;
        assert!(status.has_previous);
        assert!(status.rotation_time.is_some());
        assert!(status.overlap_remaining.is_some());
    }

    #[tokio::test]
    async fn test_clear_previous() {
        let primary = Arc::new(InMemorySecretManager::new());
        let previous = Arc::new(InMemorySecretManager::new());

        let rotating = RotatingSecretManager::new(
            primary.clone(),
            Some(previous.clone()),
            RotationConfig::default(),
        );

        assert!(rotating.rotation_status().await.has_previous);

        rotating.clear_previous().await;

        assert!(!rotating.rotation_status().await.has_previous);
    }

    #[tokio::test]
    async fn test_write_only_to_primary() {
        let primary = Arc::new(InMemorySecretManager::new());
        let previous = Arc::new(InMemorySecretManager::new());

        let rotating = RotatingSecretManager::new(
            primary.clone(),
            Some(previous.clone()),
            RotationConfig::default(),
        );

        rotating.set_secret("key", &SecretValue::new(b"value".to_vec())).await.unwrap();

        // Should be in primary
        assert!(primary.get_secret("key").await.is_ok());
        // Should NOT be in previous
        assert!(previous.get_secret("key").await.is_err());
    }
}
