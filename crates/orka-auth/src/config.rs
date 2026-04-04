use serde::Deserialize;

/// HTTP authentication configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct AuthConfig {
    /// JWT authentication configuration.
    pub jwt: Option<JwtAuthConfig>,
    /// API key entries for authentication.
    #[serde(default)]
    pub api_keys: Vec<ApiKeyEntry>,
    /// Token URL for OAuth flows.
    pub token_url: Option<String>,
    /// Authorization URL for OAuth flows.
    pub auth_url: Option<String>,
}

/// JWT authentication configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct JwtAuthConfig {
    /// HMAC secret for JWT validation (HS256).
    pub secret: Option<String>,
    /// Path to RSA public key file for JWT validation (RS256).
    pub public_key_path: Option<String>,
    /// Expected JWT audience.
    pub audience: Option<String>,
    /// Expected JWT issuer.
    pub issuer: Option<String>,
}

/// API key entry for authentication.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct ApiKeyEntry {
    /// Human-readable name for this API key.
    pub name: String,
    /// SHA-256 hash of the API key.
    pub key_hash: String,
    /// Scopes assigned to this API key.
    #[serde(default)]
    pub scopes: Vec<String>,
}

impl AuthConfig {
    /// Validate the authentication configuration.
    pub fn validate(&self) -> orka_core::Result<()> {
        if let Some(jwt) = &self.jwt
            && jwt.secret.is_none()
            && jwt.public_key_path.is_none()
        {
            return Err(orka_core::Error::Config(
                "auth.jwt: at least one of 'secret' or 'public_key_path' must be set".into(),
            ));
        }
        let mut seen_names = std::collections::HashSet::new();
        for entry in &self.api_keys {
            if entry.name.is_empty() {
                return Err(orka_core::Error::Config(
                    "auth.api_keys: key name must not be empty".into(),
                ));
            }
            if !seen_names.insert(&entry.name) {
                return Err(orka_core::Error::Config(format!(
                    "auth.api_keys: key name '{}' is not unique",
                    entry.name
                )));
            }
            if entry.key_hash.is_empty() {
                return Err(orka_core::Error::Config(format!(
                    "auth.api_keys: key '{}' has an empty key_hash",
                    entry.name
                )));
            }
        }
        Ok(())
    }
}

impl ApiKeyEntry {
    /// Create a new API key entry.
    pub fn new(name: impl Into<String>, key_hash: impl Into<String>, scopes: Vec<String>) -> Self {
        Self {
            name: name.into(),
            key_hash: key_hash.into(),
            scopes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_entry_creation() {
        let entry = ApiKeyEntry {
            name: "test-key".to_string(),
            key_hash: "abc123".to_string(),
            scopes: vec!["read".to_string(), "write".to_string()],
        };
        assert_eq!(entry.name, "test-key");
        assert_eq!(entry.scopes.len(), 2);
    }

    #[test]
    fn validate_jwt_requires_secret_or_key_path() {
        let config = AuthConfig {
            jwt: Some(JwtAuthConfig {
                secret: None,
                public_key_path: None,
                audience: None,
                issuer: None,
            }),
            api_keys: vec![],
            token_url: None,
            auth_url: None,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_jwt_with_secret_passes() {
        let config = AuthConfig {
            jwt: Some(JwtAuthConfig {
                secret: Some("my-secret".into()),
                public_key_path: None,
                audience: None,
                issuer: None,
            }),
            api_keys: vec![],
            token_url: None,
            auth_url: None,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_jwt_with_key_path_passes() {
        let config = AuthConfig {
            jwt: Some(JwtAuthConfig {
                secret: None,
                public_key_path: Some("/path/to/key.pem".into()),
                audience: None,
                issuer: None,
            }),
            api_keys: vec![],
            token_url: None,
            auth_url: None,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_key_name() {
        let config = AuthConfig {
            jwt: None,
            api_keys: vec![ApiKeyEntry::new("", "hash123", vec![])],
            token_url: None,
            auth_url: None,
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("name must not be empty"));
    }

    #[test]
    fn validate_rejects_duplicate_key_names() {
        let config = AuthConfig {
            jwt: None,
            api_keys: vec![
                ApiKeyEntry::new("my-key", "hash1", vec![]),
                ApiKeyEntry::new("my-key", "hash2", vec![]),
            ],
            token_url: None,
            auth_url: None,
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("not unique"));
    }

    #[test]
    fn validate_rejects_empty_key_hash() {
        let config = AuthConfig {
            jwt: None,
            api_keys: vec![ApiKeyEntry::new("my-key", "", vec![])],
            token_url: None,
            auth_url: None,
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("empty key_hash"));
    }

    #[test]
    fn validate_no_auth_configured_passes() {
        let config = AuthConfig::default();
        assert!(config.validate().is_ok());
    }
}
