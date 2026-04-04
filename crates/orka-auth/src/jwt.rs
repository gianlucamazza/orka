use async_trait::async_trait;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use orka_core::{Error, Result};
use serde::Deserialize;
use tracing::debug;

use crate::{
    authenticator::Authenticator,
    types::{AuthIdentity, Credentials},
};

#[derive(Debug, Deserialize)]
struct Claims {
    sub: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    scopes: Option<Vec<String>>,
    _iss: Option<String>,
    _aud: Option<serde_json::Value>,
}

/// Authenticator that validates JWTs using either an HMAC secret or an RSA
/// public key.
pub struct JwtAuthenticator {
    _issuer: String,
    _audience: Option<String>,
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
            _issuer: issuer,
            _audience: audience,
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
            _issuer: issuer,
            _audience: audience,
            decoding_key,
            validation,
        })
    }
}

#[async_trait]
impl Authenticator for JwtAuthenticator {
    async fn authenticate(&self, creds: &Credentials) -> Result<AuthIdentity> {
        let Credentials::Bearer(token) = creds else {
            return Err(Error::Auth("expected Bearer token".into()));
        };

        let token_data = decode::<Claims>(token, &self.decoding_key, &self.validation)
            .map_err(|e| Error::Auth(format!("invalid JWT: {e}")))?;

        let claims = token_data.claims;

        let principal = claims.sub.unwrap_or_else(|| "anonymous".into());

        // Extract scopes from either `scope` (space-separated string) or `scopes`
        // (array)
        let scopes = if let Some(scope_str) = claims.scope {
            scope_str
                .split_whitespace()
                .map(ToString::to_string)
                .collect()
        } else {
            claims.scopes.unwrap_or_default()
        };

        debug!(principal = %principal, scopes = ?scopes, "JWT authenticated");

        Ok(AuthIdentity { principal, scopes })
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default,
    clippy::default_trait_access,
    clippy::needless_pass_by_value,
    clippy::stable_sort_primitive
)]
mod tests {
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde::Serialize;

    use super::*;

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
        let result = auth.authenticate(&Credentials::Bearer(token)).await;
        assert!(result.is_err(), "expired JWT should be rejected");
    }

    // RSA test key pair generated with `openssl genrsa 2048` (for tests only).
    const TEST_RSA_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDV2FFDyVDiXqYO
fAc/WOUj+0ZtGj13pjuTBniXZuXo/c7ZJ+43PmHMGhHjD+SrDPEtx51U/lvUGlMs
eqL1V7Wsj5f7ntedzRqFGksorvBrpN22A/AJJ8iTA4gWd4xp/E/Kpg15Kr7Nvr9y
1O8TYP8ifIugyrh4qwSUtB0ky6Ba5L6RgA1Dpj0+xvsdM5HiCDwjsB7p+Om+E4Bn
4XrFZTNpuQ4fPr4zpdhY6DHO922Kqm94ppMdTE4Y6soLD1215iKPi9AdIWTZJwAr
rk0jvVH3ERMecTH1jBT20pMuMOIIWvU6oiUFeYu5HyHQO/kHLX22XDZuQ8d7H97Z
A11yMK2zAgMBAAECggEAEU7zcOmj/taLWUPHsGRpE4r8jcsga4VMcB+HdjWxaTTV
37TALr+BWexIQ1kfeIrRIJP2E5GM7DN/ROveMb56KW/r7MVUDWUy/s/8glv6gLP0
8A0Miikqrl+MTck6V7/A05WyJHsFu5BzXX+HpElnDSsIgCi4Wqf49HJJo5dJsOGJ
+OdrMvp/zipNTRX1dAaKbZK5+5S8s+a8m8Nei1R1M8HFpNqIX5gDrdDKdLiUDsGV
rO3GCluhGTABbY7ugj0XywKBzCRGsD3FqdIFCzzwWzYsrMidNPWMrHGO5kQvzsCE
cQwceIN+Hm5yAAGGHVWnq8b6XnXpq5FgKvLjqeavTQKBgQDwzdrKFLXx19jxUMbL
aF92yPD7ilYfhCE8ePWSe8LhbpFbYbQhBRh7ko6UhOrFvDNcGV9bnQKr7edNtBPH
EoCeQ4ttOVl/Rw+CVo4whdt+rJAQvJyLdTCFry0e0Fbu3IWk440a6dt0OPjnSQMl
8n0DAU8JPMxS5uKAo40qJ9Rl5QKBgQDjVvFv80dRw7r619N4/jt2Ul2sLazc310Q
+ELVpjj3Ej/JAbnBAmxKXxjoVZs0bDL1bPZt9Qv1SDsHR6R/leDrHJ5zOAp07coV
7Bzosx4lnevjeRpEaFXh834CQxxURm73m/8IGVneXQMlcMeM5GQttdEDIwXkIpVD
jZuGVV8LtwKBgQDPAQd+BIPMHLN/7uKV5Wl7YY3XjrouaZVwuMLSY9XJmRUXq0v/
vrOnNvuerQxtYzX7jEwvIzAywFbAs3b1APDUHFh1UoGfKmUotGOTTt67bHiECr/A
GsPViBuPi6XPvH6emoaohjSDGp7NpDQyoNvag3uAA2XaNmSsFOy7OnYaCQKBgHSj
LswZLQYuPchk4wK4rRlPuO+Vn5LSppUGSoQC/EcG/eLLF9qGu7iMgeLxyOdke+Cf
Pu+7QZ9ep6pcf3FWHEeEL2c94V+MgJouwcZB3729AEF86JUgUq/SlVvjwq0aVeSE
DJzDJPCJGAwliLwokZ1CIzJQzbz2YU5/YMPTGSiRAoGBANT2g/3dC9CGkzFflUG/
+Rp8OMXIta1TKQB63btAvLbik/hCitpQF7NzDDE646bEWuc6dIAuItIJHcMbhYJz
CWnVmzhX3cosNNhZJN+CNINeBRsE93t9R8dXO5Z0MNNkr3mJ7YunfYuPr5czzHPF
ovlAmSFOd8j/qOAY5A2oSnD/
-----END PRIVATE KEY-----";

    const TEST_RSA_PUBLIC_KEY: &str = "-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA1dhRQ8lQ4l6mDnwHP1jl
I/tGbRo9d6Y7kwZ4l2bl6P3O2SfuNz5hzBoR4w/kqwzxLcedVP5b1BpTLHqi9Ve1
rI+X+57Xnc0ahRpLKK7wa6TdtgPwCSfIkwOIFneMafxPyqYNeSq+zb6/ctTvE2D/
InyLoMq4eKsElLQdJMugWuS+kYANQ6Y9Psb7HTOR4gg8I7Ae6fjpvhOAZ+F6xWUz
abkOHz6+M6XYWOgxzvdtiqpveKaTHUxOGOrKCw9dteYij4vQHSFk2ScAK65NI71R
9xETHnEx9YwU9tKTLjDiCFr1OqIlBXmLuR8h0Dv5By19tlw2bkPHex/e2QNdcjCt
swIDAQAB
-----END PUBLIC KEY-----";

    #[tokio::test]
    async fn rsa_rs256_valid_token() {
        let auth = JwtAuthenticator::with_rsa_pem(
            "test-issuer".into(),
            None,
            TEST_RSA_PUBLIC_KEY.as_bytes(),
        )
        .unwrap();

        #[derive(Serialize)]
        struct Rs256Claims<'a> {
            sub: &'a str,
            scope: &'a str,
            iss: &'a str,
            exp: u64,
        }
        let claims = Rs256Claims {
            sub: "rsa-user",
            scope: "admin",
            iss: "test-issuer",
            exp: chrono::Utc::now().timestamp() as u64 + 3600,
        };
        let token = encode(
            &Header::new(jsonwebtoken::Algorithm::RS256),
            &claims,
            &EncodingKey::from_rsa_pem(TEST_RSA_PRIVATE_KEY.as_bytes()).unwrap(),
        )
        .unwrap();

        let identity = auth
            .authenticate(&Credentials::Bearer(token))
            .await
            .unwrap();

        assert_eq!(identity.principal, "rsa-user");
        assert_eq!(identity.scopes, vec!["admin"]);
    }

    #[tokio::test]
    async fn rsa_invalid_pem_returns_error() {
        let result =
            JwtAuthenticator::with_rsa_pem("test-issuer".into(), None, b"not-a-pem");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn jwt_with_valid_audience_passes() {
        let secret = "test-secret-key-at-least-32-bytes-long!";
        let auth = JwtAuthenticator::with_secret(
            "test-issuer".into(),
            Some("my-app".into()),
            secret,
        );

        #[derive(Serialize)]
        struct AudClaims {
            sub: String,
            scope: String,
            iss: String,
            aud: String,
            exp: u64,
        }
        let claims = AudClaims {
            sub: "user1".into(),
            scope: "read".into(),
            iss: "test-issuer".into(),
            aud: "my-app".into(),
            exp: chrono::Utc::now().timestamp() as u64 + 3600,
        };
        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();

        let identity = auth
            .authenticate(&Credentials::Bearer(token))
            .await
            .unwrap();
        assert_eq!(identity.principal, "user1");
    }

    #[tokio::test]
    async fn jwt_with_wrong_audience_rejected() {
        let secret = "test-secret-key-at-least-32-bytes-long!";
        let auth = JwtAuthenticator::with_secret(
            "test-issuer".into(),
            Some("my-app".into()),
            secret,
        );

        #[derive(Serialize)]
        struct AudClaims {
            sub: String,
            scope: String,
            iss: String,
            aud: String,
            exp: u64,
        }
        let claims = AudClaims {
            sub: "user1".into(),
            scope: "read".into(),
            iss: "test-issuer".into(),
            aud: "other-app".into(), // wrong audience
            exp: chrono::Utc::now().timestamp() as u64 + 3600,
        };
        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();

        let result = auth.authenticate(&Credentials::Bearer(token)).await;
        assert!(result.is_err(), "wrong audience should be rejected");
    }

    #[tokio::test]
    async fn jwt_scopes_as_array_extracted() {
        let secret = "test-secret-key-at-least-32-bytes-long!";
        let auth = JwtAuthenticator::with_secret("test-issuer".into(), None, secret);

        #[derive(Serialize)]
        struct ArrayScopesClaims {
            sub: String,
            scopes: Vec<String>, // array, not space-separated string
            iss: String,
            exp: u64,
        }
        let claims = ArrayScopesClaims {
            sub: "user2".into(),
            scopes: vec!["read".into(), "write".into(), "admin".into()],
            iss: "test-issuer".into(),
            exp: chrono::Utc::now().timestamp() as u64 + 3600,
        };
        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();

        let identity = auth
            .authenticate(&Credentials::Bearer(token))
            .await
            .unwrap();

        assert_eq!(identity.principal, "user2");
        assert_eq!(identity.scopes, vec!["read", "write", "admin"]);
    }
}
