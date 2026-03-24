//! System integration configuration (OS, Scheduler).

use std::path::PathBuf;

use serde::Deserialize;

use crate::{
    Error, Result,
    config::{defaults, primitives::OsPermissionLevel},
};

/// Linux OS integration configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct OsConfig {
    /// Enable OS integration.
    #[serde(default = "defaults::default_os_enabled")]
    pub enabled: bool,
    /// Permission level for OS operations.
    #[serde(default)]
    pub permission_level: OsPermissionLevel,
    /// Allowed paths for filesystem access.
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    /// Denied paths (takes precedence).
    #[serde(default)]
    pub denied_paths: Vec<String>,
    /// Allowed shell commands.
    #[serde(default)]
    pub allowed_shell_commands: Vec<String>,
    /// Coding tool orchestration policy and provider configuration.
    #[serde(default)]
    pub coding: CodingConfig,
    /// Sudo configuration.
    #[serde(default)]
    pub sudo: SudoConfig,
}

impl Default for OsConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::default_os_enabled(),
            permission_level: OsPermissionLevel::default(),
            allowed_paths: Vec::new(),
            denied_paths: Vec::new(),
            allowed_shell_commands: Vec::new(),
            coding: CodingConfig::default(),
            sudo: SudoConfig::default(),
        }
    }
}

impl OsConfig {
    /// Validate OS-related configuration.
    pub fn validate(&self) -> Result<()> {
        self.coding.validate()?;
        Ok(())
    }
}

/// Coding delegation orchestration configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct CodingConfig {
    /// Enable coding delegation.
    #[serde(default)]
    pub enabled: bool,
    /// Which coding tool should be treated as the orchestrator-selected
    /// default.
    #[serde(default = "defaults::default_coding_default_tool")]
    pub default_provider: String,
    /// Routing policy when `default_provider = "auto"`.
    #[serde(default = "defaults::default_coding_selection_policy")]
    pub selection_policy: String,
    /// Inject workspace context into delegated coding prompts.
    #[serde(default = "defaults::default_coding_inject_workspace_context")]
    pub inject_workspace_context: bool,
    /// Require a verification command for delegated coding tasks.
    #[serde(default = "defaults::default_coding_require_verification")]
    pub require_verification: bool,
    /// Allow callers to override the configured working directory.
    #[serde(default = "defaults::default_coding_allow_working_dir_override")]
    pub allow_working_dir_override: bool,
    /// Provider-specific configuration.
    #[serde(default)]
    pub providers: CodingProvidersConfig,
}

impl Default for CodingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_provider: defaults::default_coding_default_tool(),
            selection_policy: defaults::default_coding_selection_policy(),
            inject_workspace_context: defaults::default_coding_inject_workspace_context(),
            require_verification: defaults::default_coding_require_verification(),
            allow_working_dir_override: defaults::default_coding_allow_working_dir_override(),
            providers: CodingProvidersConfig::default(),
        }
    }
}

impl CodingConfig {
    /// Validate coding delegation orchestration settings.
    pub fn validate(&self) -> Result<()> {
        if !matches!(
            self.default_provider.as_str(),
            "auto" | "claude_code" | "codex"
        ) {
            return Err(Error::Config(format!(
                "os.coding.default_provider must be one of auto/claude_code/codex, got '{}'",
                self.default_provider
            )));
        }

        if !matches!(
            self.selection_policy.as_str(),
            "availability" | "prefer_claude" | "prefer_codex"
        ) {
            return Err(Error::Config(format!(
                "os.coding.selection_policy must be one of availability/prefer_claude/prefer_codex, got '{}'",
                self.selection_policy
            )));
        }

        self.providers.validate()?;

        if self.enabled && !self.providers.claude_code.enabled && !self.providers.codex.enabled {
            return Err(Error::Config(
                "os.coding.enabled requires at least one enabled provider under os.coding.providers"
                    .into(),
            ));
        }

        Ok(())
    }
}

/// Coding delegation provider configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct CodingProvidersConfig {
    /// Claude Code backend configuration.
    #[serde(default)]
    pub claude_code: ClaudeCodeConfig,
    /// Codex backend configuration.
    #[serde(default)]
    pub codex: CodexConfig,
}

impl CodingProvidersConfig {
    /// Validate provider configuration.
    pub fn validate(&self) -> Result<()> {
        self.claude_code.validate()?;
        self.codex.validate()?;
        Ok(())
    }
}

/// Claude Code provider configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct ClaudeCodeConfig {
    /// Enable Claude Code integration.
    #[serde(default = "defaults::default_claude_code_enabled")]
    pub enabled: bool,
    /// Path to Claude Code executable.
    pub executable_path: Option<PathBuf>,
    /// Default model override.
    pub model: Option<String>,
    /// Maximum turn count for the delegated run.
    pub max_turns: Option<u32>,
    /// Execution timeout in seconds.
    #[serde(default = "defaults::default_coding_timeout_secs")]
    pub timeout_secs: u64,
    /// Extra system prompt appended to the delegated run.
    pub append_system_prompt: Option<String>,
    /// Explicit allow-list for Claude Code tools.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Allow Claude Code to modify files.
    #[serde(default)]
    pub allow_file_modifications: bool,
    /// Allow Claude Code to execute commands.
    #[serde(default)]
    pub allow_command_execution: bool,
}

impl Default for ClaudeCodeConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::default_claude_code_enabled(),
            executable_path: None,
            model: None,
            max_turns: None,
            timeout_secs: defaults::default_coding_timeout_secs(),
            append_system_prompt: None,
            allowed_tools: Vec::new(),
            allow_file_modifications: false,
            allow_command_execution: false,
        }
    }
}

impl ClaudeCodeConfig {
    /// Validate Claude Code provider configuration.
    pub fn validate(&self) -> Result<()> {
        if self.timeout_secs == 0 {
            return Err(Error::Config(
                "os.coding.providers.claude_code.timeout_secs must be greater than zero".into(),
            ));
        }
        Ok(())
    }
}

/// Codex provider configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct CodexConfig {
    /// Enable Codex integration.
    #[serde(default = "defaults::default_codex_enabled")]
    pub enabled: bool,
    /// Path to Codex executable.
    pub executable_path: Option<PathBuf>,
    /// Default model override.
    pub model: Option<String>,
    /// Execution timeout in seconds.
    #[serde(default = "defaults::default_coding_timeout_secs")]
    pub timeout_secs: u64,
    /// Sandbox mode passed to Codex exec.
    pub sandbox_mode: Option<String>,
    /// Approval policy passed to Codex.
    pub approval_policy: Option<String>,
    /// Allow Codex to modify files.
    #[serde(default)]
    pub allow_file_modifications: bool,
    /// Allow Codex to execute commands.
    #[serde(default)]
    pub allow_command_execution: bool,
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::default_codex_enabled(),
            executable_path: None,
            model: None,
            timeout_secs: defaults::default_coding_timeout_secs(),
            sandbox_mode: None,
            approval_policy: None,
            allow_file_modifications: false,
            allow_command_execution: false,
        }
    }
}

impl CodexConfig {
    /// Validate Codex provider configuration.
    pub fn validate(&self) -> Result<()> {
        if self.timeout_secs == 0 {
            return Err(Error::Config(
                "os.coding.providers.codex.timeout_secs must be greater than zero".into(),
            ));
        }

        if let Some(mode) = self.sandbox_mode.as_deref()
            && !matches!(mode, "read-only" | "workspace-write" | "danger-full-access")
        {
            return Err(Error::Config(format!(
                "os.coding.providers.codex.sandbox_mode must be one of read-only/workspace-write/danger-full-access, got '{}'",
                mode
            )));
        }

        if let Some(policy) = self.approval_policy.as_deref()
            && !matches!(policy, "untrusted" | "on-failure" | "on-request" | "never")
        {
            return Err(Error::Config(format!(
                "os.coding.providers.codex.approval_policy must be one of untrusted/on-failure/on-request/never, got '{}'",
                policy
            )));
        }

        Ok(())
    }
}

/// Sudo configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct SudoConfig {
    /// Allow sudo elevation.
    #[serde(default = "defaults::default_sudo_allowed")]
    pub allowed: bool,
    /// Allowed sudo commands (empty = all allowed if sudo enabled).
    #[serde(default)]
    pub allowed_commands: Vec<String>,
    /// Require password for sudo.
    #[serde(default = "default_password_required")]
    pub password_required: bool,
}

impl Default for SudoConfig {
    fn default() -> Self {
        Self {
            allowed: defaults::default_sudo_allowed(),
            allowed_commands: Vec::new(),
            password_required: default_password_required(),
        }
    }
}

const fn default_password_required() -> bool {
    true
}

/// Cron scheduler configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct SchedulerConfig {
    /// Enable scheduler.
    #[serde(default = "defaults::default_scheduler_enabled")]
    pub enabled: bool,
    /// Scheduled jobs.
    #[serde(default)]
    pub jobs: Vec<ScheduledJob>,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::default_scheduler_enabled(),
            jobs: Vec::new(),
        }
    }
}

/// Scheduled job definition.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct ScheduledJob {
    /// Job name.
    pub name: String,
    /// Cron expression.
    pub schedule: String,
    /// Command to execute.
    pub command: String,
    /// Workspace to run in.
    pub workspace: Option<String>,
    /// Enable this job.
    #[serde(default = "default_job_enabled")]
    pub enabled: bool,
}

const fn default_job_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coding_validation_requires_provider_when_enabled() {
        let mut config = CodingConfig {
            enabled: true,
            ..Default::default()
        };
        config.providers.claude_code.enabled = false;
        config.providers.codex.enabled = false;

        assert!(config.validate().is_err());
    }

    #[test]
    fn coding_validation_accepts_enabled_provider() {
        let mut config = CodingConfig {
            enabled: true,
            ..Default::default()
        };
        config.providers.codex.enabled = true;

        assert!(config.validate().is_ok());
    }

    #[test]
    fn coding_validation_rejects_invalid_default_provider() {
        let config = CodingConfig {
            default_provider: "invalid".into(),
            ..Default::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn codex_validation_rejects_invalid_sandbox_mode() {
        let config = CodexConfig {
            enabled: true,
            sandbox_mode: Some("invalid".into()),
            ..Default::default()
        };

        assert!(config.validate().is_err());
    }
}
