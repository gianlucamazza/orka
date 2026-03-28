//! Policy enforcement for git agent operations.

use std::sync::LazyLock;

use crate::{GitAuthorshipMode, GitConfig, error::GitError};

/// Compiled conventional-commits regex, initialised once at first use.
#[allow(clippy::expect_used)]
static CONVENTIONAL_COMMIT_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"^(feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)(\([^)]+\))?!?: .+",
    )
    .expect("static regex is valid")
});

/// Enforces git safety policies derived from [`GitConfig`].
///
/// The guard is created once from config and shared (via `Arc`) across all
/// skill instances.  Its methods are synchronous and cheap — they only inspect
/// in-memory configuration, never touch the filesystem.
#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct GitGuard {
    protected_branches: Vec<String>,
    allow_force_push: bool,
    require_conventional_commits: bool,
    sign_commits: bool,
    secret_patterns: Vec<glob::Pattern>,
    allowed_remotes: Vec<String>,
    pub(crate) max_diff_lines: usize,
    pub(crate) max_log_entries: usize,
    pub(crate) command_timeout_secs: u64,
    /// Authorship mode (used by commit skill).
    pub(crate) authorship_mode: GitAuthorshipMode,
    /// Authorship display name.
    pub(crate) authorship_name: String,
    /// Authorship email.
    pub(crate) authorship_email: String,
    pub(crate) sign_commits_flag: bool,
}

impl GitGuard {
    /// Build a guard from the given config.
    ///
    /// # Errors
    /// Returns an error if any of the `secret_patterns` are invalid globs.
    pub fn from_config(config: &GitConfig) -> Result<Self, GitError> {
        let secret_patterns = config
            .secret_patterns
            .iter()
            .map(|p| {
                glob::Pattern::new(p)
                    .map_err(|e| GitError::InvalidArg(format!("invalid secret_pattern '{p}': {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            protected_branches: config.protected_branches.clone(),
            allow_force_push: config.allow_force_push,
            require_conventional_commits: config.require_conventional_commits,
            sign_commits: config.sign_commits,
            secret_patterns,
            allowed_remotes: config.allowed_remotes.clone(),
            max_diff_lines: config.max_diff_lines,
            max_log_entries: config.max_log_entries,
            command_timeout_secs: config.command_timeout_secs,
            authorship_mode: config.authorship.mode.clone(),
            authorship_name: config.authorship.name.clone(),
            authorship_email: config.authorship.email.clone(),
            sign_commits_flag: config.sign_commits,
        })
    }

    /// Returns `true` if any file path in `files` matches a secret pattern.
    ///
    /// # Errors
    /// Returns [`GitError::Policy`] if a secret file is detected.
    pub fn check_no_secrets(&self, files: &[&str]) -> Result<(), GitError> {
        for file in files {
            // Match against the filename only, not the full path
            let filename = std::path::Path::new(file)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(file);
            for pat in &self.secret_patterns {
                if pat.matches(file) || pat.matches(filename) {
                    return Err(GitError::Policy(format!(
                        "refusing to commit '{file}': matches secret pattern '{}'",
                        pat.as_str()
                    )));
                }
            }
        }
        Ok(())
    }

    /// Validates that a push to `branch` with the given `force` flag is
    /// allowed.
    ///
    /// # Errors
    /// Returns [`GitError::Policy`] if the push is blocked.
    pub fn check_push_allowed(&self, branch: &str, force: bool) -> Result<(), GitError> {
        // Force-push check (global)
        if force && !self.allow_force_push {
            return Err(GitError::Policy(format!(
                "force-push to '{branch}' is disabled (set git.allow_force_push = true to override)"
            )));
        }

        // Protected-branch check: always blocks force-push, blocks direct push too
        for pattern in &self.protected_branches {
            let pat = glob::Pattern::new(pattern).map_err(|_| {
                GitError::Policy(format!("invalid protected_branch pattern '{pattern}'"))
            })?;
            if pat.matches(branch) {
                if force {
                    return Err(GitError::Policy(format!(
                        "force-push to protected branch '{branch}' is never allowed"
                    )));
                }
                // Non-force push to protected branch is also blocked by default
                return Err(GitError::Policy(format!(
                    "direct push to protected branch '{branch}' is not allowed (open a PR instead)"
                )));
            }
        }
        Ok(())
    }

    /// Validates that a commit message follows Conventional Commits format.
    ///
    /// # Errors
    /// Returns [`GitError::Policy`] if the message is not valid.
    pub fn check_commit_message(&self, message: &str) -> Result<(), GitError> {
        if !self.require_conventional_commits {
            return Ok(());
        }
        if !CONVENTIONAL_COMMIT_RE.is_match(message.trim()) {
            return Err(GitError::Policy(format!(
                "commit message does not follow Conventional Commits format.\n\
                 Expected: type(scope): description\n\
                 Types: feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert\n\
                 Got: {message:?}"
            )));
        }
        Ok(())
    }

    /// Checks that the remote name is in the allowlist.
    ///
    /// # Errors
    /// Returns [`GitError::Policy`] if the remote is not permitted.
    pub fn check_remote(&self, remote: &str) -> Result<(), GitError> {
        if self.allowed_remotes.is_empty() {
            return Ok(());
        }
        if !self.allowed_remotes.iter().any(|r| r == remote) {
            return Err(GitError::Policy(format!(
                "remote '{remote}' is not in git.allowed_remotes: {:?}",
                self.allowed_remotes
            )));
        }
        Ok(())
    }

    /// Returns `true` if commit signing is required.
    pub fn sign_commits(&self) -> bool {
        self.sign_commits
    }

    /// Builds the authorship trailer or `--author` argument for a commit,
    /// based on the configured [`GitAuthorshipMode`].
    ///
    /// Returns `None` when mode is `None`.
    pub fn authorship_args(&self) -> Option<AuthorshipArgs> {
        match self.authorship_mode {
            GitAuthorshipMode::Trailer => Some(AuthorshipArgs::Trailer {
                name: self.authorship_name.clone(),
                email: self.authorship_email.clone(),
            }),
            GitAuthorshipMode::Author => Some(AuthorshipArgs::Author {
                name: self.authorship_name.clone(),
                email: self.authorship_email.clone(),
            }),
            // GitAuthorshipMode::None and forward-compatible unknown modes
            _ => None,
        }
    }
}

/// How the agent's authorship should be recorded in a commit.
#[derive(Debug, Clone)]
pub enum AuthorshipArgs {
    /// Append a `Co-Authored-By:` trailer to the commit message body.
    Trailer {
        /// Display name.
        name: String,
        /// Email.
        email: String,
    },
    /// Set `--author` on the commit command.
    Author {
        /// Display name.
        name: String,
        /// Email.
        email: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard() -> GitGuard {
        GitGuard::from_config(&GitConfig::default()).expect("default config builds guard")
    }

    #[test]
    fn conventional_commit_valid() {
        let g = guard();
        assert!(
            g.check_commit_message("feat(auth): add OAuth2 support")
                .is_ok()
        );
        assert!(
            g.check_commit_message("fix: correct off-by-one in parser")
                .is_ok()
        );
        assert!(
            g.check_commit_message("chore!: drop support for node 12")
                .is_ok()
        );
    }

    #[test]
    fn conventional_commit_invalid() {
        let g = guard();
        assert!(g.check_commit_message("Added stuff").is_err());
        assert!(g.check_commit_message("WIP").is_err());
    }

    #[test]
    fn secret_detection() {
        let g = guard();
        assert!(g.check_no_secrets(&["src/main.rs"]).is_ok());
        assert!(g.check_no_secrets(&[".env"]).is_err());
        assert!(g.check_no_secrets(&["config/prod.env"]).is_err());
        assert!(g.check_no_secrets(&["keys/server.key"]).is_err());
        assert!(g.check_no_secrets(&["cert.pem"]).is_err());
    }

    #[test]
    fn push_policy_blocks_protected() {
        let g = guard();
        assert!(g.check_push_allowed("feat/new-thing", false).is_ok());
        assert!(g.check_push_allowed("main", false).is_err());
        assert!(g.check_push_allowed("feat/x", true).is_err()); // force disabled
    }

    #[test]
    fn remote_allowlist_empty_means_all() {
        let g = guard();
        assert!(g.check_remote("origin").is_ok());
        assert!(g.check_remote("github").is_ok());
    }

    #[test]
    fn remote_allowlist_enforced() {
        let mut config = GitConfig::default();
        config.allowed_remotes = vec!["origin".to_string()];
        let g = GitGuard::from_config(&config).unwrap();
        assert!(g.check_remote("origin").is_ok());
        assert!(g.check_remote("github").is_err());
    }
}
