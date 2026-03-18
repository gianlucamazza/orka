//! Authentication and authorization for API and WebSocket endpoints.
//!
//! - [`Authenticator`] — async trait for credential verification
//! - [`ApiKeyAuthenticator`], [`JwtAuthenticator`] — built-in implementations
//! - [`AuthLayer`] / [`AuthService`] — tower middleware for request authentication

#![warn(missing_docs)]

/// SHA-256 API key authenticator.
pub mod api_key;
/// `Authenticator` trait definition.
pub mod authenticator;
/// JWT authenticator (HMAC / RSA).
pub mod jwt;
/// Tower middleware that performs authentication on incoming requests.
pub mod middleware;
/// Identity and credentials types.
pub mod types;

#[cfg(feature = "test-util")]
pub mod testing;

pub use api_key::ApiKeyAuthenticator;
pub use authenticator::Authenticator;
pub use jwt::JwtAuthenticator;
pub use middleware::{AuthLayer, AuthService};
pub use types::{AuthIdentity, Credentials};

#[cfg(feature = "test-util")]
pub use testing::InMemoryAuthenticator;
