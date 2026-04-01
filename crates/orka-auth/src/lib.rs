//! Authentication and authorization for API and WebSocket endpoints.
//!
//! - [`Authenticator`] — async trait for credential verification
//! - [`ApiKeyAuthenticator`], [`JwtAuthenticator`] — built-in implementations
//! - [`AuthLayer`] / [`AuthService`] — tower middleware for request
//!   authentication

#![warn(missing_docs)]

/// SHA-256 API key authenticator.
pub mod api_key;
/// `Authenticator` trait definition.
pub mod authenticator;
/// Authentication configuration owned by `orka-auth`.
pub mod config;
/// JWT authenticator (HMAC / RSA).
pub mod jwt;
/// Tower middleware that performs authentication on incoming requests.
pub mod middleware;
/// Identity and credentials types.
pub mod types;

/// In-memory test doubles for authentication (available with `test-util`
/// feature or `#[cfg(test)]`).
#[cfg(any(feature = "test-util", test))]
pub mod testing;

pub use api_key::ApiKeyAuthenticator;
pub use authenticator::{Authenticator, CompositeAuthenticator};
pub use config::{ApiKeyEntry, AuthConfig, JwtAuthConfig};
pub use jwt::JwtAuthenticator;
pub use middleware::{AuthLayer, AuthService};
#[cfg(any(feature = "test-util", test))]
pub use testing::InMemoryAuthenticator;
pub use types::{AuthIdentity, Credentials};
