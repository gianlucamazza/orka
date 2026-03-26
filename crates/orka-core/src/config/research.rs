//! Autonomous research campaign configuration.

use serde::Deserialize;

use crate::{Error, Result, config::defaults};

/// Native research subsystem configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct ResearchConfig {
    /// Enable native research campaigns and related API/CLI surface.
    pub enabled: bool,
    /// Require explicit approval for candidate promotion.
    pub require_promotion_approval: bool,
    /// Branch patterns that always require approval even if the global policy
    /// is relaxed.
    pub protected_target_branches: Vec<String>,
}

impl Default for ResearchConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::default_research_enabled(),
            require_promotion_approval: defaults::default_research_require_promotion_approval(),
            protected_target_branches: defaults::default_research_protected_target_branches(),
        }
    }
}

impl ResearchConfig {
    /// Validate research configuration values.
    pub fn validate(&self) -> Result<()> {
        for pattern in &self.protected_target_branches {
            if pattern.trim().is_empty() {
                return Err(Error::Config(
                    "research.protected_target_branches must not contain empty patterns".into(),
                ));
            }
            glob::Pattern::new(pattern).map_err(|e| {
                Error::Config(format!(
                    "research.protected_target_branches contains invalid glob pattern '{pattern}': {e}"
                ))
            })?;
        }
        Ok(())
    }
}
