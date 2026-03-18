use async_trait::async_trait;

use crate::types::{AuthIdentity, Credentials};

/// Trait for authentication backends.
#[async_trait]
pub trait Authenticator: Send + Sync + 'static {
    /// Verify the given credentials and return the established identity.
    async fn authenticate(&self, creds: &Credentials) -> orka_core::Result<AuthIdentity>;
}
