use async_trait::async_trait;
use std::sync::Arc;

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

        Err(last_error.unwrap_or_else(|| orka_core::Error::Auth("no auth backends configured".into())))
    }
}
