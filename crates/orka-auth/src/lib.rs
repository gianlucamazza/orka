//! Authentication and authorization for API and WebSocket endpoints.
//!
//! - [`Authenticator`] — async trait for credential verification
//! - [`ApiKeyAuthenticator`], [`JwtAuthenticator`] — built-in implementations
//! - [`AuthLayer`] / [`AuthService`] — tower middleware for request authentication

#![warn(missing_docs)]

#[allow(missing_docs)]
pub mod api_key;
#[allow(missing_docs)]
pub mod authenticator;
#[allow(missing_docs)]
pub mod jwt;
#[allow(missing_docs)]
pub mod middleware;
#[allow(missing_docs)]
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
