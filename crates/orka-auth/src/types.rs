use serde::{Deserialize, Serialize};

/// Identity established after authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthIdentity {
    /// Unique identifier of the authenticated entity (e.g. API key name, JWT
    /// subject).
    pub principal: String,
    /// Permission scopes granted to this identity.
    pub scopes: Vec<String>,
    /// Mobile device ID extracted from the `dvc` JWT claim, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

impl AuthIdentity {
    /// Create an anonymous (unauthenticated) identity with no scopes.
    pub fn anonymous() -> Self {
        Self {
            principal: "anonymous".into(),
            scopes: vec![],
            device_id: None,
        }
    }
}

/// Credentials extracted from a request.
#[derive(Debug, Clone)]
pub enum Credentials {
    /// Raw API key extracted from the `X-Api-Key` header.
    ApiKey(String),
    /// Bearer token extracted from the `Authorization: Bearer` header.
    Bearer(String),
    /// No credentials present in the request.
    None,
}
