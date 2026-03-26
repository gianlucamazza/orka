//! Security configuration (Authentication, Secrets, Sandbox).

use std::collections::HashMap;

use serde::{Deserialize, Deserializer};

use crate::config::defaults;

/// HTTP authentication configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct AuthConfig {
    /// JWT authentication configuration.
    pub jwt: Option<JwtAuthConfig>,
    /// API key entries for authentication.
    #[serde(default)]
    pub api_keys: Vec<ApiKeyEntry>,
    /// Token URL for OAuth flows.
    pub token_url: Option<String>,
    /// Authorization URL for OAuth flows.
    pub auth_url: Option<String>,
}

/// JWT authentication configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct JwtAuthConfig {
    /// HMAC secret for JWT validation (HS256).
    pub secret: Option<String>,
    /// Path to RSA public key file for JWT validation (RS256).
    pub public_key_path: Option<String>,
    /// Expected JWT audience.
    pub audience: Option<String>,
    /// Expected JWT issuer.
    pub issuer: Option<String>,
}

/// API key entry for authentication.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct ApiKeyEntry {
    /// Human-readable name for this API key.
    pub name: String,
    /// SHA-256 hash of the API key.
    pub key_hash: String,
    /// Scopes assigned to this API key.
    #[serde(default)]
    pub scopes: Vec<String>,
}

impl ApiKeyEntry {
    /// Create a new API key entry.
    pub fn new(name: impl Into<String>, key_hash: impl Into<String>, scopes: Vec<String>) -> Self {
        Self {
            name: name.into(),
            key_hash: key_hash.into(),
            scopes,
        }
    }
}

/// Secret storage backend selection.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum SecretBackend {
    /// Redis-backed secret storage (default).
    #[default]
    Redis,
    /// File-backed secret storage — no external infrastructure required.
    /// Suitable for local development and initial setup.
    File,
}

/// Secret storage configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct SecretConfig {
    /// Secret storage backend.
    #[serde(default)]
    pub backend: SecretBackend,
    /// Path for file-backed storage (when `backend = "file"`).
    /// Defaults to `~/.config/orka/secrets.json`.
    pub file_path: Option<String>,
    /// Path to the master encryption key (hex-encoded, 32 bytes).
    pub encryption_key_path: Option<String>,
    /// Environment variable containing the encryption key.
    pub encryption_key_env: Option<String>,
    /// Redis configuration for secret storage.
    #[serde(flatten)]
    pub redis: super::infrastructure::RedisConfig,
}

/// Code sandbox configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct SandboxConfig {
    /// Sandbox backend to use.
    #[serde(default = "defaults::default_sandbox_backend")]
    pub backend: String,
    /// Resource limits for sandboxed processes.
    #[serde(default)]
    pub limits: SandboxLimitsConfig,
    /// Allowed paths for filesystem access.
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    /// Denied paths (takes precedence over `allowed_paths`).
    #[serde(default)]
    pub denied_paths: Vec<String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            backend: defaults::default_sandbox_backend(),
            limits: SandboxLimitsConfig::default(),
            allowed_paths: Vec::new(),
            denied_paths: Vec::new(),
        }
    }
}

/// Resource limits for sandboxed processes.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct SandboxLimitsConfig {
    /// Maximum execution time in seconds.
    #[serde(default = "defaults::default_timeout_secs")]
    pub timeout_secs: u64,
    /// Maximum memory usage in bytes.
    #[serde(default = "defaults::default_max_memory_bytes")]
    pub max_memory_bytes: usize,
    /// Maximum output size in bytes.
    #[serde(default = "defaults::default_max_output_bytes")]
    pub max_output_bytes: usize,
    /// Maximum number of open file descriptors.
    #[serde(default)]
    pub max_open_files: Option<usize>,
    /// Maximum number of processes.
    #[serde(default = "default_max_pids")]
    pub max_pids: usize,
}

impl Default for SandboxLimitsConfig {
    fn default() -> Self {
        Self {
            timeout_secs: defaults::default_timeout_secs(),
            max_memory_bytes: defaults::default_max_memory_bytes(),
            max_output_bytes: defaults::default_max_output_bytes(),
            max_open_files: None,
            max_pids: default_max_pids(),
        }
    }
}

const fn default_max_pids() -> usize {
    10
}

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
    fn sandbox_default_limits() {
        let config = SandboxConfig::default();
        assert_eq!(config.limits.timeout_secs, 30);
        assert_eq!(config.limits.max_memory_bytes, 64 * 1024 * 1024);
        assert_eq!(config.limits.max_pids, 10);
    }

    #[test]
    fn api_key_entry_creation() {
        let entry = ApiKeyEntry {
            name: "test-key".to_string(),
            key_hash: "abc123".to_string(),
            scopes: vec!["read".to_string(), "write".to_string()],
        };
        assert_eq!(entry.name, "test-key");
        assert_eq!(entry.scopes.len(), 2);
    }

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
}
