mod guard;
mod skills;

use std::sync::Arc;

use orka_core::config::HttpClientConfig;
use orka_core::traits::Skill;
use orka_core::Result;
use tracing::info;

pub use guard::SsrfGuard;

/// Create HTTP skills from config.
pub fn create_http_skills(config: &HttpClientConfig) -> Result<Vec<Arc<dyn Skill>>> {
    let guard = Arc::new(SsrfGuard::new(config.blocked_domains.clone()));

    let request_skill: Arc<dyn Skill> = Arc::new(skills::request::HttpRequestSkill::new(
        guard,
        config.max_response_bytes,
        config.default_timeout_secs,
        &config.user_agent,
    )?);

    info!("HTTP skills initialized (http_request)");
    Ok(vec![request_skill])
}
