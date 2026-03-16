use serde::{Deserialize, Serialize};

/// Identity established after authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthIdentity {
    pub principal: String,
    pub scopes: Vec<String>,
}

impl AuthIdentity {
    pub fn anonymous() -> Self {
        Self {
            principal: "anonymous".into(),
            scopes: vec![],
        }
    }
}

/// Credentials extracted from a request.
#[derive(Debug, Clone)]
pub enum Credentials {
    ApiKey(String),
    Bearer(String),
    None,
}
