use crate::defaults::*;
use serde::{Deserialize, Serialize};

/// Configuration for the system prompt pipeline.
///
/// Controls section ordering, separators, and limits.
///
/// # Example
///
/// ```toml
/// [prompts.pipeline]
/// sections = ["persona", "datetime", "workspace", "tools", "principles"]
/// section_separator = "\n\n"
/// max_principles = 5
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// Section order (by name).
    #[serde(default = "default_section_order_vec")]
    pub sections: Vec<String>,

    /// Separator between sections.
    #[serde(default = "default_separator")]
    pub section_separator: String,

    /// Maximum number of principles to include.
    #[serde(default = "default_max_principles_usize")]
    pub max_principles: usize,

    /// Include conversation summary if available.
    #[serde(default = "default_true")]
    pub include_summary: bool,

    /// Include current datetime.
    #[serde(default = "default_true")]
    pub include_datetime: bool,

    /// Timezone for datetime formatting.
    #[serde(default = "default_timezone")]
    pub timezone: String,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            sections: default_section_order_vec(),
            section_separator: default_separator(),
            max_principles: default_max_principles_usize(),
            include_summary: true,
            include_datetime: true,
            timezone: default_timezone(),
        }
    }
}

fn default_section_order_vec() -> Vec<String> {
    DEFAULT_SECTION_ORDER
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn default_separator() -> String {
    SECTION_SEPARATOR.to_string()
}

fn default_max_principles_usize() -> usize {
    DEFAULT_MAX_PRINCIPLES
}

fn default_true() -> bool {
    true
}

fn default_timezone() -> String {
    DEFAULT_TIMEZONE.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = PipelineConfig::default();
        assert_eq!(config.sections.len(), DEFAULT_SECTION_ORDER.len());
        assert_eq!(config.section_separator, SECTION_SEPARATOR);
        assert_eq!(config.max_principles, DEFAULT_MAX_PRINCIPLES);
        assert!(config.include_summary);
        assert!(config.include_datetime);
    }

    #[test]
    fn test_deserialize_config() {
        let toml = r#"
            sections = ["persona", "tools"]
            section_separator = "\n---\n"
            max_principles = 3
            include_summary = false
        "#;

        let config: PipelineConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.sections, vec!["persona", "tools"]);
        assert_eq!(config.section_separator, "\n---\n");
        assert_eq!(config.max_principles, 3);
        assert!(!config.include_summary);
    }
}
