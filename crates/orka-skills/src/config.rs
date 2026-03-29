use std::collections::HashMap;

use serde::{Deserialize, Deserializer};

/// WASM plugin configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct PluginConfig {
    /// Directory containing WASM plugins.
    pub dir: Option<String>,
    /// Plugin capabilities.
    #[serde(default)]
    pub capabilities: PluginCapabilities,
    /// Per-plugin configuration.
    #[serde(default)]
    pub plugins: HashMap<String, PluginInstanceConfig>,
}

impl PluginConfig {
    /// Validate plugin configuration.
    pub fn validate(&self) -> orka_core::Result<()> {
        Ok(())
    }
}

/// Plugin capabilities (deny-by-default).
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct PluginCapabilities {
    /// Host filesystem paths to pre-open for the plugin.
    ///
    /// Accepts either a boolean shorthand or an explicit path list:
    /// - `true`  → equivalent to `["."]` (current working directory)
    /// - `false` → empty list (no filesystem access)
    /// - `["/data", "/tmp"]` → explicit list of allowed paths
    ///
    /// Default: empty (deny all).
    #[serde(default, deserialize_with = "deserialize_fs_paths")]
    pub filesystem: Vec<String>,
    /// Allow outbound TCP/UDP network access.
    ///
    /// Default: `false`.
    #[serde(default)]
    pub network: bool,
    /// Environment variable names the plugin is allowed to read from the host.
    ///
    /// Only the listed variables are injected; unlisted variables are invisible
    /// to the plugin. Default: empty (deny all).
    #[serde(default)]
    pub env: Vec<String>,
}

/// Per-plugin instance configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct PluginInstanceConfig {
    /// Whether this plugin is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Capability overrides for this plugin.
    ///
    /// When present, each field replaces the corresponding global default from
    /// `[plugins.capabilities]`. Omitted fields fall back to the global value.
    #[serde(default)]
    pub capabilities: Option<PluginCapabilities>,
    /// Arbitrary plugin-specific key-value configuration.
    ///
    /// Passed through to the plugin at runtime; Orka does not interpret these
    /// values. Configure under `[plugins.plugins.<name>.config]` in TOML.
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
}

const fn default_true() -> bool {
    true
}

/// Soft skill (SKILL.md) configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct SoftSkillConfig {
    /// Directory to scan for soft skill subdirectories containing SKILL.md
    /// files.
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

impl SoftSkillConfig {
    /// Validate soft skill configuration.
    pub fn validate(&self) -> orka_core::Result<()> {
        if !matches!(self.selection_mode.as_str(), "all" | "keyword") {
            return Err(orka_core::Error::Config(format!(
                "soft_skills.selection_mode must be \"all\" or \"keyword\", got \"{}\"",
                self.selection_mode
            )));
        }
        Ok(())
    }
}

fn default_soft_skill_selection_mode() -> String {
    "all".to_string()
}

/// Deserialize `filesystem` as either a `bool` (backward-compat shorthand) or
/// an explicit `Vec<String>` list of host paths.
fn deserialize_fs_paths<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum BoolOrPaths {
        Bool(bool),
        Paths(Vec<String>),
    }

    match BoolOrPaths::deserialize(deserializer)? {
        BoolOrPaths::Bool(true) => Ok(vec![".".to_string()]),
        BoolOrPaths::Bool(false) => Ok(vec![]),
        BoolOrPaths::Paths(paths) => Ok(paths),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_capabilities_default_denies_all() {
        let caps = PluginCapabilities::default();
        assert!(caps.filesystem.is_empty());
        assert!(!caps.network);
        assert!(caps.env.is_empty());
    }

    #[test]
    fn deserialize_fs_paths_from_bool_true() -> Result<(), toml::de::Error> {
        let caps: PluginCapabilities = toml::from_str("filesystem = true\nnetwork = false")?;
        assert_eq!(caps.filesystem, vec!["."]);
        Ok(())
    }

    #[test]
    fn deserialize_fs_paths_from_bool_false() -> Result<(), toml::de::Error> {
        let caps: PluginCapabilities = toml::from_str("filesystem = false\nnetwork = false")?;
        assert!(caps.filesystem.is_empty());
        Ok(())
    }

    #[test]
    fn deserialize_fs_paths_from_list() -> Result<(), toml::de::Error> {
        let caps: PluginCapabilities = toml::from_str(r#"filesystem = ["/data", "/tmp"]"#)?;
        assert_eq!(caps.filesystem, vec!["/data", "/tmp"]);
        Ok(())
    }

    #[test]
    fn plugin_capabilities_with_env() -> Result<(), toml::de::Error> {
        let caps: PluginCapabilities = toml::from_str(r#"env = ["API_KEY", "DB_URL"]"#)?;
        assert_eq!(caps.env, vec!["API_KEY", "DB_URL"]);
        Ok(())
    }

    #[test]
    fn plugin_instance_enabled_by_default() -> Result<(), toml::de::Error> {
        let config: PluginInstanceConfig = toml::from_str("")?;
        assert!(config.enabled);
        assert!(config.capabilities.is_none());
        assert!(config.config.is_empty());
        Ok(())
    }

    #[test]
    fn plugin_instance_with_capability_overrides() -> Result<(), toml::de::Error> {
        let config: PluginInstanceConfig = toml::from_str(
            r#"
enabled = true
[capabilities]
network = true
env = ["SPECIAL_VAR"]
"#,
        )?;
        let Some(caps) = config.capabilities else {
            panic!("expected capabilities overrides");
        };
        assert!(caps.network);
        assert_eq!(caps.env, vec!["SPECIAL_VAR"]);
        assert!(caps.filesystem.is_empty());
        Ok(())
    }

    #[test]
    fn plugin_instance_without_overrides() -> Result<(), toml::de::Error> {
        let config: PluginInstanceConfig = toml::from_str("enabled = true")?;
        assert!(config.capabilities.is_none());
        Ok(())
    }

    #[test]
    fn soft_skill_config_defaults_to_all_selection() -> Result<(), toml::de::Error> {
        let config: SoftSkillConfig = toml::from_str("")?;
        assert!(config.dir.is_none());
        assert_eq!(config.selection_mode, "all");
        Ok(())
    }
}
