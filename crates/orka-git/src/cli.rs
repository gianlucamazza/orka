//! Type-safe wrapper for invoking the `git` CLI.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use tokio::process::Command;

use crate::error::GitError;

/// Output of a git CLI command.
#[derive(Debug)]
pub struct CmdOutput {
    /// Exit code. `0` means success.
    pub exit_code: i32,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

impl CmdOutput {
    /// Returns `Ok(self)` if exit code is 0, otherwise
    /// `Err(GitError::Command)`.
    pub fn into_result(self) -> Result<Self, GitError> {
        if self.exit_code == 0 {
            Ok(self)
        } else {
            Err(GitError::Command(format!(
                "git exited with code {}: {}",
                self.exit_code,
                self.stderr.trim()
            )))
        }
    }
}

/// Runs a single `git` command in the given working directory.
///
/// The `args` slice must NOT include the `git` binary itself.
///
/// # Errors
/// Returns [`GitError::Timeout`] if the command exceeds `timeout_secs`,
/// [`GitError::Command`] if spawning or waiting fails.
pub async fn run_git(
    work_dir: &Path,
    args: &[&str],
    timeout_secs: u64,
) -> Result<CmdOutput, GitError> {
    let start = std::time::Instant::now();

    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(work_dir);
    // Ensure output is machine-readable (no pager, consistent locale)
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("LANG", "en_US.UTF-8");
    cmd.env("LC_ALL", "en_US.UTF-8");
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let child = cmd
        .spawn()
        .map_err(|e| GitError::Command(format!("failed to spawn git: {e}")))?;

    let result =
        tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output()).await;

    let duration_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(Ok(out)) => Ok(CmdOutput {
            exit_code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            duration_ms,
        }),
        Ok(Err(e)) => Err(GitError::Command(format!("git execution failed: {e}"))),
        Err(_) => Err(GitError::Timeout { secs: timeout_secs }),
    }
}

/// Resolves a `cwd` argument to a concrete path.
///
/// Falls back to the current process directory when `cwd` is `None`.
///
/// # Errors
/// Returns [`GitError::InvalidArg`] if the directory does not exist.
pub fn resolve_work_dir(cwd: Option<&str>) -> Result<PathBuf, GitError> {
    match cwd {
        Some(p) => {
            let path = PathBuf::from(p);
            if path.is_dir() {
                Ok(path)
            } else {
                Err(GitError::InvalidArg(format!(
                    "cwd '{p}' is not a directory"
                )))
            }
        }
        None => std::env::current_dir().map_err(GitError::Io),
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default,
    clippy::default_trait_access,
    clippy::needless_pass_by_value,
    clippy::stable_sort_primitive
)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn git_version_succeeds() {
        let dir = std::env::current_dir().unwrap();
        let out = run_git(&dir, &["--version"], 10).await.unwrap();
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("git"));
    }

    #[tokio::test]
    async fn bad_subcommand_returns_nonzero() {
        let dir = std::env::current_dir().unwrap();
        let out = run_git(&dir, &["this-does-not-exist-xyz"], 10)
            .await
            .unwrap();
        assert_ne!(out.exit_code, 0);
        let result = out.into_result();
        assert!(result.is_err());
    }
}
