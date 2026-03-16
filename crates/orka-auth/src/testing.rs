use std::collections::HashMap;

use async_trait::async_trait;

use crate::authenticator::Authenticator;
use crate::types::{AuthIdentity, Credentials};

/// In-memory authenticator for testing. Stores plain API keys mapped to identities.
pub struct InMemoryAuthenticator {
    keys: HashMap<String, AuthIdentity>,
}

impl InMemoryAuthenticator {
    pub fn new() -> Self {
        Self {
            keys: HashMap::new(),
        }
    }

    pub fn with_key(mut self, key: &str, name: &str, scopes: Vec<String>) -> Self {
        self.keys.insert(
            key.to_owned(),
            AuthIdentity {
                principal: name.into(),
                scopes,
            },
        );
        self
    }
}

impl Default for InMemoryAuthenticator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Authenticator for InMemoryAuthenticator {
    async fn authenticate(&self, creds: &Credentials) -> orka_core::Result<AuthIdentity> {
        match creds {
            Credentials::ApiKey(key) => self
                .keys
                .get(key)
                .cloned()
                .ok_or_else(|| orka_core::Error::Auth("invalid API key".into())),
            Credentials::Bearer(_) => {
                Err(orka_core::Error::Auth("Bearer tokens not supported in test authenticator".into()))
            }
            Credentials::None => Err(orka_core::Error::Auth("missing API key".into())),
        }
    }
}
