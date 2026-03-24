//! HTTP client skills with SSRF protection.
//!
//! Provides an `http_request` skill guarded by [`SsrfGuard`] to prevent
//! server-side request forgery against internal networks.

#![warn(missing_docs)]

mod guard;
mod skills;

use std::sync::Arc;

use orka_core::Result;
use orka_core::config::HttpClientConfig;
use orka_core::traits::Skill;
use tracing::info;

pub use guard::SsrfGuard;

/// Create HTTP skills from config.
pub fn create_http_skills(config: &HttpClientConfig) -> Result<Vec<Arc<dyn Skill>>> {
    // SSRF protection with default blocked domains (AWS metadata, etc.)
    let blocked_domains = vec![
        "169.254.169.254".to_string(), // AWS metadata
        "metadata.google.internal".to_string(),
        "100.100.100.200".to_string(), // Alibaba metadata
    ];
    let guard = Arc::new(SsrfGuard::new(blocked_domains));

    let request_skill: Arc<dyn Skill> = Arc::new(skills::request::HttpRequestSkill::new(
        guard,
        10 * 1024 * 1024, // Default 10MB max response
        config.timeout_secs,
        &config.user_agent.clone().unwrap_or_else(|| "Orka/1.0".to_string()),
    )?);

    info!("HTTP skills initialized (http_request)");
    Ok(vec![request_skill])
}
