use serde::Deserialize;

const fn default_session_ttl_secs() -> u64 {
    86_400
}

/// Session store configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct SessionConfig {
    /// Session time-to-live in seconds.
    #[serde(default = "default_session_ttl_secs")]
    pub ttl_secs: u64,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            ttl_secs: default_session_ttl_secs(),
        }
    }
}

impl SessionConfig {
    /// Validate the session configuration.
    pub fn validate(&self) -> orka_core::Result<()> {
        if self.ttl_secs == 0 {
            return Err(orka_core::Error::Config(
                "session.ttl_secs must be greater than 0".into(),
            ));
        }
        Ok(())
    }
}
