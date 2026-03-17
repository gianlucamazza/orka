use std::collections::HashMap;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use orka_core::Error;
use orka_core::config::ApiKeyEntry;

use crate::authenticator::Authenticator;
use crate::types::{AuthIdentity, Credentials};

/// Authenticator that validates API keys by comparing SHA-256 hashes.
pub struct ApiKeyAuthenticator {
    /// Maps key_hash → (name, scopes)
    keys: HashMap<String, (String, Vec<String>)>,
}

impl ApiKeyAuthenticator {
    pub fn new(entries: &[ApiKeyEntry]) -> Self {
        let keys = entries
            .iter()
            .map(|e| (e.key_hash.clone(), (e.name.clone(), e.scopes.clone())))
            .collect();
        Self { keys }
    }

    fn hash_key(raw: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(raw.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

#[async_trait]
impl Authenticator for ApiKeyAuthenticator {
    async fn authenticate(&self, creds: &Credentials) -> orka_core::Result<AuthIdentity> {
        match creds {
            Credentials::ApiKey(key) => {
                let hash = Self::hash_key(key);
                match self.keys.get(&hash) {
                    Some((name, scopes)) => Ok(AuthIdentity {
                        principal: name.clone(),
                        scopes: scopes.clone(),
                    }),
                    None => Err(Error::Auth("invalid API key".into())),
                }
            }
            Credentials::Bearer(_) => Err(Error::Auth("expected API key, got Bearer token".into())),
            Credentials::None => Err(Error::Auth("missing API key".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(name: &str, raw_key: &str, scopes: Vec<String>) -> ApiKeyEntry {
        ApiKeyEntry {
            name: name.into(),
            key_hash: ApiKeyAuthenticator::hash_key(raw_key),
            scopes,
        }
    }

    #[tokio::test]
    async fn valid_key_authenticates() {
        let entries = vec![make_entry("test-client", "secret123", vec!["read".into()])];
        let auth = ApiKeyAuthenticator::new(&entries);

        let identity = auth
            .authenticate(&Credentials::ApiKey("secret123".into()))
            .await
            .unwrap();

        assert_eq!(identity.principal, "test-client");
        assert_eq!(identity.scopes, vec!["read"]);
    }

    #[tokio::test]
    async fn wrong_key_fails() {
        let entries = vec![make_entry("test-client", "secret123", vec![])];
        let auth = ApiKeyAuthenticator::new(&entries);

        let result = auth
            .authenticate(&Credentials::ApiKey("wrong-key".into()))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn missing_key_fails() {
        let entries = vec![make_entry("test-client", "secret123", vec![])];
        let auth = ApiKeyAuthenticator::new(&entries);

        let result = auth.authenticate(&Credentials::None).await;

        assert!(result.is_err());
    }
}
