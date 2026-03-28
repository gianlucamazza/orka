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
}
