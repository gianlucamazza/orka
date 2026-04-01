use serde::Deserialize;

const fn default_gateway_rate_limit() -> u32 {
    0
}

const fn default_gateway_dedup_ttl_secs() -> u64 {
    300
}

/// API gateway rate limiting and deduplication configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    /// Redis URL for deduplication and rate limiting (optional).
    #[serde(default)]
    pub redis_url: Option<String>,
    /// Maximum requests per minute per session (0 = unlimited).
    #[serde(default = "default_gateway_rate_limit")]
    pub rate_limit: u32,
    /// Deduplication TTL in seconds.
    #[serde(default = "default_gateway_dedup_ttl_secs")]
    pub dedup_ttl_secs: u64,
    /// Enable request deduplication.
    #[serde(default)]
    pub dedup_enabled: bool,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            redis_url: None,
            rate_limit: default_gateway_rate_limit(),
            dedup_ttl_secs: default_gateway_dedup_ttl_secs(),
            dedup_enabled: false,
        }
    }
}

impl GatewayConfig {
    /// Validate the gateway configuration.
    pub fn validate(&self) -> orka_core::Result<()> {
        Ok(())
    }
}
