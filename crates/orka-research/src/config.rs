//! Autonomous research campaign configuration.

use serde::Deserialize;

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
            enabled: default_research_enabled(),
            require_promotion_approval: default_research_require_promotion_approval(),
            protected_target_branches: default_research_protected_target_branches(),
        }
    }
}

const fn default_research_enabled() -> bool {
    false
}

const fn default_research_require_promotion_approval() -> bool {
    true
}

fn default_research_protected_target_branches() -> Vec<String> {
    vec!["main".to_string(), "master".to_string()]
}

impl ResearchConfig {
    /// Validate research configuration values.
    pub fn validate(&self) -> orka_core::Result<()> {
        for pattern in &self.protected_target_branches {
            if pattern.trim().is_empty() {
                return Err(orka_core::Error::Config(
                    "research.protected_target_branches must not contain empty patterns".into(),
                ));
            }
            glob::Pattern::new(pattern).map_err(|e| {
                orka_core::Error::Config(format!(
                    "research.protected_target_branches contains invalid glob pattern '{pattern}': {e}"
                ))
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::ResearchConfig;

    #[test]
    fn research_defaults_match_expected_policy() {
        let config = ResearchConfig::default();

        assert!(!config.enabled);
        assert!(config.require_promotion_approval);
        assert_eq!(config.protected_target_branches, ["main", "master"]);
    }
}
