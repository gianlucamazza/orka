//! System integration configuration (OS, Scheduler).

use crate::config::defaults;
use crate::config::primitives::OsPermissionLevel;
use serde::Deserialize;
use std::path::PathBuf;

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
    /// Claude Code integration.
    #[serde(default)]
    pub claude_code: ClaudeCodeConfig,
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
            claude_code: ClaudeCodeConfig::default(),
            sudo: SudoConfig::default(),
        }
    }
}

/// Claude Code integration configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct ClaudeCodeConfig {
    /// Enable Claude Code integration.
    #[serde(default = "defaults::default_claude_code_enabled")]
    pub enabled: bool,
    /// Path to Claude Code executable.
    pub executable_path: Option<PathBuf>,
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
            allow_file_modifications: false,
            allow_command_execution: false,
        }
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
