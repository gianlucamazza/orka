//! System integration configuration (OS, Scheduler).

use std::{fmt, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    config::{defaults, primitives::OsPermissionLevel},
    Error, Result,
};

/// Coding orchestration provider selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingProvider {
    /// Choose provider automatically based on availability or policy.
    #[default]
    Auto,
    /// Prefer Claude Code as the coding backend.
    ClaudeCode,
    /// Prefer Codex as the coding backend.
    Codex,
    /// Prefer `OpenCode` as the coding backend.
    #[serde(rename = "opencode")]
    OpenCode,
}

impl fmt::Display for CodingProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => f.write_str("auto"),
            Self::ClaudeCode => f.write_str("claude_code"),
            Self::Codex => f.write_str("codex"),
            Self::OpenCode => f.write_str("opencode"),
        }
    }
}

/// Routing policy when `default_provider` is `Auto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingSelectionPolicy {
    /// Prefer whichever provider is available; fall back in order: claude → codex → opencode.
    #[default]
    Availability,
    /// Try Claude Code first, fall back to `Codex` then `OpenCode`.
    PreferClaude,
    /// Try Codex first, fall back to Claude Code then `OpenCode`.
    PreferCodex,
    /// Try `OpenCode` first, fall back to Claude Code then `Codex`.
    PreferOpenCode,
}

impl fmt::Display for CodingSelectionPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Availability => f.write_str("availability"),
            Self::PreferClaude => f.write_str("prefer_claude"),
            Self::PreferCodex => f.write_str("prefer_codex"),
            Self::PreferOpenCode => f.write_str("prefer_opencode"),
        }
    }
}

/// Codex sandbox isolation level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxMode {
    /// Read-only filesystem access.
    ReadOnly,
    /// Allow writes inside the workspace directory.
    WorkspaceWrite,
    /// Full filesystem access — use with caution.
    DangerFullAccess,
}

impl fmt::Display for SandboxMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadOnly => f.write_str("read-only"),
            Self::WorkspaceWrite => f.write_str("workspace-write"),
            Self::DangerFullAccess => f.write_str("danger-full-access"),
        }
    }
}

/// Codex approval policy for executing commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalPolicy {
    /// Treat all commands as untrusted; prompt for every execution.
    Untrusted,
    /// Prompt only when a command fails.
    OnFailure,
    /// Prompt when the user explicitly requests it.
    OnRequest,
    /// Never prompt; approve all commands automatically.
    Never,
}

impl fmt::Display for ApprovalPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Untrusted => f.write_str("untrusted"),
            Self::OnFailure => f.write_str("on-failure"),
            Self::OnRequest => f.write_str("on-request"),
            Self::Never => f.write_str("never"),
        }
    }
}

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
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct CodingConfig {
    /// Enable coding delegation.
    #[serde(default)]
    pub enabled: bool,
    /// Which coding tool should be treated as the orchestrator-selected
    /// default.
    #[serde(default)]
    pub default_provider: CodingProvider,
    /// Routing policy when `default_provider` is `Auto`.
    #[serde(default)]
    pub selection_policy: CodingSelectionPolicy,
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
            default_provider: CodingProvider::default(),
            selection_policy: CodingSelectionPolicy::default(),
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
        self.providers.validate()?;

        if self.enabled
            && !self.providers.claude_code.enabled
            && !self.providers.codex.enabled
            && !self.providers.opencode.enabled
        {
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
    /// `OpenCode` backend configuration.
    #[serde(default)]
    pub opencode: OpenCodeConfig,
}

impl CodingProvidersConfig {
    /// Validate provider configuration.
    pub fn validate(&self) -> Result<()> {
        self.claude_code.validate()?;
        self.codex.validate()?;
        self.opencode.validate()?;
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
    pub sandbox_mode: Option<SandboxMode>,
    /// Approval policy passed to Codex.
    pub approval_policy: Option<ApprovalPolicy>,
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
        Ok(())
    }
}

/// `OpenCode` provider configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct OpenCodeConfig {
    /// Enable `OpenCode` integration.
    #[serde(default = "defaults::default_opencode_enabled")]
    pub enabled: bool,
    /// Path to `OpenCode` executable.
    pub executable_path: Option<PathBuf>,
    /// Default model override in `provider/model` format (e.g. `anthropic/claude-sonnet-4-6`).
    pub model: Option<String>,
    /// Agent name passed via `--agent`.
    pub agent: Option<String>,
    /// Model variant for reasoning effort (e.g. `high`, `max`, `minimal`).
    pub variant: Option<String>,
    /// Execution timeout in seconds.
    #[serde(default = "defaults::default_coding_timeout_secs")]
    pub timeout_secs: u64,
    /// Allow `OpenCode` to modify files.
    #[serde(default)]
    pub allow_file_modifications: bool,
    /// Allow `OpenCode` to execute commands.
    #[serde(default)]
    pub allow_command_execution: bool,
}

impl Default for OpenCodeConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::default_opencode_enabled(),
            executable_path: None,
            model: None,
            agent: None,
            variant: None,
            timeout_secs: defaults::default_coding_timeout_secs(),
            allow_file_modifications: false,
            allow_command_execution: false,
        }
    }
}

impl OpenCodeConfig {
    /// Validate `OpenCode` provider configuration.
    pub fn validate(&self) -> Result<()> {
        if self.timeout_secs == 0 {
            return Err(Error::Config(
                "os.coding.providers.opencode.timeout_secs must be greater than zero".into(),
            ));
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
    /// How often (in seconds) to poll for due tasks.
    #[serde(default = "defaults::default_scheduler_poll_interval_secs")]
    pub poll_interval_secs: u64,
    /// Maximum number of tasks to execute concurrently.
    #[serde(default = "defaults::default_scheduler_max_concurrent")]
    pub max_concurrent: usize,
    /// Scheduled jobs.
    #[serde(default)]
    pub jobs: Vec<ScheduledJob>,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::default_scheduler_enabled(),
            poll_interval_secs: defaults::default_scheduler_poll_interval_secs(),
            max_concurrent: defaults::default_scheduler_max_concurrent(),
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
        config.providers.opencode.enabled = false;

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
    fn coding_provider_serde_roundtrip() {
        let toml = r#"default_provider = "claude_code""#;
        let config: CodingConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.default_provider, CodingProvider::ClaudeCode);
    }

    #[test]
    fn coding_provider_rejects_invalid() {
        let toml = r#"default_provider = "invalid_value""#;
        assert!(toml::from_str::<CodingConfig>(toml).is_err());
    }

    #[test]
    fn sandbox_mode_serde_roundtrip() {
        let toml = r#"sandbox_mode = "workspace-write""#;
        let config: CodexConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.sandbox_mode, Some(SandboxMode::WorkspaceWrite));
    }

    #[test]
    fn sandbox_mode_rejects_invalid() {
        let toml = r#"sandbox_mode = "invalid""#;
        assert!(toml::from_str::<CodexConfig>(toml).is_err());
    }

    #[test]
    fn approval_policy_serde_roundtrip() {
        let toml = r#"approval_policy = "on-failure""#;
        let config: CodexConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.approval_policy, Some(ApprovalPolicy::OnFailure));
    }

    #[test]
    fn coding_provider_display() {
        assert_eq!(CodingProvider::Auto.to_string(), "auto");
        assert_eq!(CodingProvider::ClaudeCode.to_string(), "claude_code");
        assert_eq!(CodingProvider::Codex.to_string(), "codex");
        assert_eq!(CodingProvider::OpenCode.to_string(), "opencode");
    }

    #[test]
    fn coding_provider_opencode_serde_roundtrip() {
        let toml = r#"default_provider = "opencode""#;
        let config: CodingConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.default_provider, CodingProvider::OpenCode);
    }

    #[test]
    fn coding_validation_accepts_opencode_provider() {
        let mut config = CodingConfig {
            enabled: true,
            ..Default::default()
        };
        config.providers.opencode.enabled = true;

        assert!(config.validate().is_ok());
    }

    #[test]
    fn opencode_config_serde_roundtrip() {
        let toml = r#"
            enabled = true
            model = "anthropic/claude-sonnet-4-6"
            agent = "default"
            variant = "high"
            timeout_secs = 600
            allow_file_modifications = true
            allow_command_execution = true
        "#;
        let config: OpenCodeConfig = toml::from_str(toml).unwrap();
        assert!(config.enabled);
        assert_eq!(config.model.as_deref(), Some("anthropic/claude-sonnet-4-6"));
        assert_eq!(config.agent.as_deref(), Some("default"));
        assert_eq!(config.variant.as_deref(), Some("high"));
        assert_eq!(config.timeout_secs, 600);
        assert!(config.allow_file_modifications);
        assert!(config.allow_command_execution);
    }

    #[test]
    fn opencode_config_rejects_zero_timeout() {
        let toml = r#"timeout_secs = 0"#;
        let config: OpenCodeConfig = toml::from_str(toml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn selection_policy_prefer_opencode_display() {
        assert_eq!(
            CodingSelectionPolicy::PreferOpenCode.to_string(),
            "prefer_opencode"
        );
    }
}
