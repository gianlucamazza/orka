pub mod api_key;
pub mod authenticator;
pub mod jwt;
pub mod middleware;
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
