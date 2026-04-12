use serde::Deserialize;

fn default_sandbox_backend() -> String {
    "process".to_string()
}

const fn default_timeout_secs() -> u64 {
    30
}

const fn default_max_memory_bytes() -> usize {
    64 * 1024 * 1024
}

const fn default_max_output_bytes() -> usize {
    1024 * 1024
}

const fn default_max_pids() -> usize {
    10
}

/// Code sandbox configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct SandboxConfig {
    /// Sandbox backend to use.
    #[serde(default = "default_sandbox_backend")]
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
            backend: default_sandbox_backend(),
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
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Maximum memory usage in bytes.
    #[serde(default = "default_max_memory_bytes")]
    pub max_memory_bytes: usize,
    /// Maximum output size in bytes.
    #[serde(default = "default_max_output_bytes")]
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
            timeout_secs: default_timeout_secs(),
            max_memory_bytes: default_max_memory_bytes(),
            max_output_bytes: default_max_output_bytes(),
            max_open_files: None,
            max_pids: default_max_pids(),
        }
    }
}

impl SandboxConfig {
    /// Validate the sandbox configuration.
    pub fn validate(&self) -> orka_core::Result<()> {
        if self.limits.timeout_secs == 0 {
            return Err(orka_core::Error::Config(
                "sandbox.limits.timeout_secs must be greater than 0".into(),
            ));
        }
        if self.limits.max_memory_bytes == 0 {
            return Err(orka_core::Error::Config(
                "sandbox.limits.max_memory_bytes must be greater than 0".into(),
            ));
        }
        if self.limits.max_pids == 0 {
            return Err(orka_core::Error::Config(
                "sandbox.limits.max_pids must be greater than 0".into(),
            ));
        }
        Ok(())
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
}
