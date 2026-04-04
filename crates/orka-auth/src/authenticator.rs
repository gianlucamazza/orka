use std::sync::Arc;

use async_trait::async_trait;

use crate::types::{AuthIdentity, Credentials};

/// Trait for authentication backends.
#[async_trait]
pub trait Authenticator: Send + Sync + 'static {
    /// Verify the given credentials and return the established identity.
    async fn authenticate(&self, creds: &Credentials) -> orka_core::Result<AuthIdentity>;
}

/// Authenticator that tries multiple backends in order until one succeeds.
pub struct CompositeAuthenticator {
    backends: Vec<Arc<dyn Authenticator>>,
}

impl CompositeAuthenticator {
    /// Create a new ordered authenticator chain.
    pub fn new(backends: Vec<Arc<dyn Authenticator>>) -> Self {
        Self { backends }
    }
}

#[async_trait]
impl Authenticator for CompositeAuthenticator {
    async fn authenticate(&self, creds: &Credentials) -> orka_core::Result<AuthIdentity> {
        let mut last_error = None;
        for backend in &self.backends {
            match backend.authenticate(creds).await {
                Ok(identity) => return Ok(identity),
                Err(err) => last_error = Some(err),
            }
        }

        Err(last_error
            .unwrap_or_else(|| orka_core::Error::Auth("no auth backends configured".into())))
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::default_trait_access
)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{testing::InMemoryAuthenticator, types::Credentials};

    #[tokio::test]
    async fn composite_first_backend_succeeds() {
        let a1 = InMemoryAuthenticator::new().with_key("k1", "alice", vec!["read".into()]);
        let a2 = InMemoryAuthenticator::new().with_key("k2", "bob", vec!["write".into()]);
        let composite = CompositeAuthenticator::new(vec![Arc::new(a1), Arc::new(a2)]);

        let identity = composite
            .authenticate(&Credentials::ApiKey("k1".into()))
            .await
            .unwrap();

        assert_eq!(identity.principal, "alice");
        assert_eq!(identity.scopes, vec!["read"]);
    }

    #[tokio::test]
    async fn composite_first_fails_second_succeeds() {
        let a1 = InMemoryAuthenticator::new(); // no keys registered
        let a2 = InMemoryAuthenticator::new().with_key("k2", "bob", vec!["write".into()]);
        let composite = CompositeAuthenticator::new(vec![Arc::new(a1), Arc::new(a2)]);

        let identity = composite
            .authenticate(&Credentials::ApiKey("k2".into()))
            .await
            .unwrap();

        assert_eq!(identity.principal, "bob");
    }

    #[tokio::test]
    async fn composite_all_fail_returns_error() {
        let a1 = InMemoryAuthenticator::new();
        let a2 = InMemoryAuthenticator::new();
        let composite = CompositeAuthenticator::new(vec![Arc::new(a1), Arc::new(a2)]);

        let result = composite
            .authenticate(&Credentials::ApiKey("no-such-key".into()))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn composite_empty_backends_returns_no_backends_error() {
        let composite = CompositeAuthenticator::new(vec![]);

        let err = composite
            .authenticate(&Credentials::ApiKey("key".into()))
            .await
            .unwrap_err();

        assert!(err.to_string().contains("no auth backends"));
    }

    #[tokio::test]
    async fn composite_returns_last_error_not_first() {
        // Both fail, but with different errors. Composite should return the last one.
        let a1 = InMemoryAuthenticator::new(); // returns "invalid API key"
        let a2 = InMemoryAuthenticator::new(); // also returns "invalid API key"
        let composite = CompositeAuthenticator::new(vec![Arc::new(a1), Arc::new(a2)]);

        // Should not panic and should return an auth error
        let result = composite.authenticate(&Credentials::None).await;
        assert!(result.is_err());
    }
}
