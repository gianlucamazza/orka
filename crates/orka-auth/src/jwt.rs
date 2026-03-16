use async_trait::async_trait;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use orka_core::{Error, Result};
use serde::Deserialize;
use tracing::debug;

use crate::authenticator::Authenticator;
use crate::types::{AuthIdentity, Credentials};

#[derive(Debug, Deserialize)]
struct Claims {
    sub: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    scopes: Option<Vec<String>>,
    #[allow(dead_code)]
    iss: Option<String>,
    #[allow(dead_code)]
    aud: Option<serde_json::Value>,
}

pub struct JwtAuthenticator {
    #[allow(dead_code)]
    issuer: String,
    #[allow(dead_code)]
    audience: Option<String>,
    decoding_key: DecodingKey,
    validation: Validation,
}

impl JwtAuthenticator {
    /// Create from a static HMAC secret (HS256).
    pub fn with_secret(issuer: String, audience: Option<String>, secret: &str) -> Self {
        let decoding_key = DecodingKey::from_secret(secret.as_bytes());
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[&issuer]);
        if let Some(ref aud) = audience {
            validation.set_audience(&[aud]);
        }
        validation.validate_exp = true;
        validation.leeway = 10; // 10 seconds clock skew tolerance

        Self {
            issuer,
            audience,
            decoding_key,
            validation,
        }
    }

    /// Create from an RSA public key PEM (RS256).
    pub fn with_rsa_pem(issuer: String, audience: Option<String>, pem: &[u8]) -> Result<Self> {
        let decoding_key = DecodingKey::from_rsa_pem(pem)
            .map_err(|e| Error::Auth(format!("invalid RSA PEM: {e}")))?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&issuer]);
        if let Some(ref aud) = audience {
            validation.set_audience(&[aud]);
        }
        validation.validate_exp = true;
        validation.leeway = 10; // 10 seconds clock skew tolerance

        Ok(Self {
            issuer,
            audience,
            decoding_key,
            validation,
        })
    }
}

#[async_trait]
impl Authenticator for JwtAuthenticator {
    async fn authenticate(&self, creds: &Credentials) -> Result<AuthIdentity> {
        let token = match creds {
            Credentials::Bearer(t) => t,
            _ => return Err(Error::Auth("expected Bearer token".into())),
        };

        let token_data = decode::<Claims>(token, &self.decoding_key, &self.validation)
            .map_err(|e| Error::Auth(format!("invalid JWT: {e}")))?;

        let claims = token_data.claims;

        let principal = claims.sub.unwrap_or_else(|| "anonymous".into());

        // Extract scopes from either `scope` (space-separated string) or `scopes` (array)
        let scopes = if let Some(scope_str) = claims.scope {
            scope_str
                .split_whitespace()
                .map(|s| s.to_string())
                .collect()
        } else {
            claims.scopes.unwrap_or_default()
        };

        debug!(principal = %principal, scopes = ?scopes, "JWT authenticated");

        Ok(AuthIdentity { principal, scopes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde::Serialize;

    #[derive(Serialize)]
    struct TestClaims {
        sub: String,
        scope: String,
        iss: String,
        exp: u64,
    }

    fn make_token(secret: &str, sub: &str, scope: &str, issuer: &str) -> String {
        let claims = TestClaims {
            sub: sub.into(),
            scope: scope.into(),
            iss: issuer.into(),
            exp: chrono::Utc::now().timestamp() as u64 + 3600,
        };
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn valid_jwt() {
        let auth = JwtAuthenticator::with_secret(
            "test-issuer".into(),
            None,
            "test-secret-key-at-least-32-bytes-long!",
        );
        let token = make_token(
            "test-secret-key-at-least-32-bytes-long!",
            "user123",
            "read write",
            "test-issuer",
        );
        let identity = auth
            .authenticate(&Credentials::Bearer(token))
            .await
            .unwrap();
        assert_eq!(identity.principal, "user123");
        assert_eq!(identity.scopes, vec!["read", "write"]);
    }

    #[tokio::test]
    async fn invalid_jwt() {
        let auth = JwtAuthenticator::with_secret(
            "test-issuer".into(),
            None,
            "test-secret-key-at-least-32-bytes-long!",
        );
        let result = auth
            .authenticate(&Credentials::Bearer("invalid.token.here".into()))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn wrong_credentials_type() {
        let auth = JwtAuthenticator::with_secret("test-issuer".into(), None, "secret");
        let result = auth.authenticate(&Credentials::ApiKey("key".into())).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn expired_jwt_rejected() {
        let secret = "test-secret-key-at-least-32-bytes-long!";
        let auth = JwtAuthenticator::with_secret("test-issuer".into(), None, secret);
        let claims = TestClaims {
            sub: "user123".into(),
            scope: "read".into(),
            iss: "test-issuer".into(),
            exp: 1_000_000, // far in the past
        };
        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();
        let result = auth
            .authenticate(&Credentials::Bearer(token))
            .await;
        assert!(result.is_err(), "expired JWT should be rejected");
    }
}
