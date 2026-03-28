//! Prompt template configuration.

use serde::Deserialize;

/// Prompt template configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct PromptsConfig {
    /// Directory containing custom templates (relative to workspace or
    /// absolute).
    #[serde(default = "default_prompts_dir")]
    pub templates_dir: String,
    /// Enable hot-reload of templates.
    #[serde(default = "default_true")]
    pub hot_reload: bool,
    /// Default section order for system prompts.
    #[serde(default)]
    pub section_order: Vec<String>,
    /// Separator between sections.
    #[serde(default = "default_section_separator")]
    pub section_separator: String,
    /// Maximum principles to include.
    #[serde(default = "default_max_principles")]
    pub max_principles: usize,
}

impl Default for PromptsConfig {
    fn default() -> Self {
        Self {
            templates_dir: default_prompts_dir(),
            hot_reload: default_true(),
            section_order: vec![
                "persona".to_string(),
                "datetime".to_string(),
                "workspace".to_string(),
                "tools".to_string(),
                "principles".to_string(),
                "summary".to_string(),
            ],
            section_separator: default_section_separator(),
            max_principles: default_max_principles(),
        }
    }
}

fn default_prompts_dir() -> String {
    "PROMPTS".to_string()
}

fn default_section_separator() -> String {
    "\n\n".to_string()
}

const fn default_max_principles() -> usize {
    5
}

const fn default_true() -> bool {
    true
}
