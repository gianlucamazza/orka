//! Tools, skills, and plugin configuration.

use serde::Deserialize;
use std::collections::HashMap;

/// Tool enable/disable configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct ToolsConfig {
    /// Globally allowed tools.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Globally denied tools (takes precedence).
    #[serde(default)]
    pub deny: Vec<String>,
    /// Tool-specific configuration.
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
}

/// Soft skill selection mode.
///
/// - `"all"` (default): inject every registered soft skill into every request.
/// - `"keyword"`: inject only soft skills whose name or tags match words in the
///   user's message. Reduces prompt bloat when many soft skills are registered.
fn default_soft_skill_selection_mode() -> String {
    "all".to_string()
}

/// Soft skill (SKILL.md) configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct SoftSkillConfig {
    /// Directory to scan for soft skill subdirectories containing SKILL.md files.
    pub dir: Option<String>,
    /// How to select which soft skills to inject: `"all"` or `"keyword"`.
    #[serde(default = "default_soft_skill_selection_mode")]
    pub selection_mode: String,
}

impl Default for SoftSkillConfig {
    fn default() -> Self {
        Self {
            dir: None,
            selection_mode: default_soft_skill_selection_mode(),
        }
    }
}
