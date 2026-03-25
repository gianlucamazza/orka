//! Git skills configuration.

use serde::{Deserialize, Serialize};

/// Top-level git integration configuration.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[non_exhaustive]
pub struct GitConfig {
    /// Enable git skills.
    pub enabled: bool,
    /// Branches that agents cannot push to directly.
    /// Supports glob patterns (e.g. `"release/*"`).
    pub protected_branches: Vec<String>,
    /// Whether force-push is allowed. Defaults to `false`.
    pub allow_force_push: bool,
    /// Enforce Conventional Commits format on every agent commit.
    pub require_conventional_commits: bool,
    /// Pass `-S` to `git commit` to enable commit signing.
    /// When `false` (default), signing follows the local git config.
    pub sign_commits: bool,
    /// Glob patterns for file paths that must never be committed
    /// (e.g. `".env"`, `"*.key"`).
    pub secret_patterns: Vec<String>,
    /// Whitelist of allowed remote names (e.g. `["origin"]`).
    /// An empty list allows all remotes.
    pub allowed_remotes: Vec<String>,
    /// Maximum number of diff lines returned by `git_diff`. Default 5000.
    pub max_diff_lines: usize,
    /// Maximum number of log entries returned by `git_log`. Default 100.
    pub max_log_entries: usize,
    /// Timeout in seconds for git CLI commands. Default 60.
    pub command_timeout_secs: u64,
    /// AI authorship attribution appended to agent commits.
    pub authorship: GitAuthorshipConfig,
    /// Git worktree management configuration.
    pub worktree: GitWorktreeConfig,
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            protected_branches: vec!["main".to_string(), "release/*".to_string()],
            allow_force_push: false,
            require_conventional_commits: true,
            sign_commits: false,
            secret_patterns: vec![
                ".env".to_string(),
                "*.env".to_string(),
                "*.key".to_string(),
                "*.pem".to_string(),
                "*.p12".to_string(),
                "credentials*".to_string(),
            ],
            allowed_remotes: Vec::new(),
            max_diff_lines: 5_000,
            max_log_entries: 100,
            command_timeout_secs: 60,
            authorship: GitAuthorshipConfig::default(),
            worktree: GitWorktreeConfig::default(),
        }
    }
}

/// How the agent's authorship is attributed in commits.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum GitAuthorshipMode {
    /// Append a `Co-Authored-By:` trailer to the commit message.
    #[default]
    Trailer,
    /// Override the commit author via `--author`.
    Author,
    /// Do not add any authorship attribution.
    None,
}

/// Authorship attribution configuration for agent commits.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[non_exhaustive]
pub struct GitAuthorshipConfig {
    /// Authorship mode.
    pub mode: GitAuthorshipMode,
    /// Display name used in the trailer or author override.
    pub name: String,
    /// Email used in the trailer or author override.
    pub email: String,
}

impl Default for GitAuthorshipConfig {
    fn default() -> Self {
        Self {
            mode: GitAuthorshipMode::Trailer,
            name: "orka-agent".to_string(),
            email: "agent@orka.local".to_string(),
        }
    }
}

/// Worktree management configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[non_exhaustive]
pub struct GitWorktreeConfig {
    /// Directory under the repo root where worktrees are created.
    pub base_dir: String,
    /// Files that are hard-copied into each new worktree.
    pub copy_files: Vec<String>,
    /// Directories that are symlinked into each new worktree (read-only
    /// caches).
    pub symlink_dirs: Vec<String>,
    /// Maximum number of concurrent worktrees. Default 10.
    pub max_concurrent: usize,
}

impl Default for GitWorktreeConfig {
    fn default() -> Self {
        Self {
            base_dir: ".orka-worktrees".to_string(),
            copy_files: vec![".env".to_string()],
            symlink_dirs: vec![".fastembed_cache".to_string()],
            max_concurrent: 10,
        }
    }
}
