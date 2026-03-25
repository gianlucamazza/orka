//! Git worktree lifecycle management for agent isolation.
//!
//! Each agent task gets its own worktree — a fully isolated working tree that
//! shares the `.git` object database with the main repo.  This pattern (used
//! by Claude Code and Dagger Container Use) gives per-agent isolation at a
//! fraction of the cost of a full clone.
//!
//! **Cost vs. full clone**
//! 5 worktrees on a 200 MB repo ≈ 200 MB total (vs. ~1 GB for 5 clones).

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use tracing::{info, warn};

use crate::{cli::run_git, error::GitError};

/// Metadata written into every worktree as `.orka-worktree-meta.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeMeta {
    /// Slug-style name (derived from branch).
    pub name: String,
    /// Branch checked out in this worktree.
    pub branch: String,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Agent ID that owns this worktree, if set.
    pub agent_id: Option<String>,
}

/// Summary of a live worktree.
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    /// Slug name.
    pub name: String,
    /// Absolute path to the worktree directory.
    pub path: PathBuf,
    /// Branch checked out.
    pub branch: String,
    /// HEAD commit SHA (short).
    pub head_sha: String,
    /// Creation time from metadata, if available.
    pub created_at: Option<DateTime<Utc>>,
    /// Owning agent ID, if set.
    pub agent_id: Option<String>,
}

/// Manages git worktrees under a single repository.
#[derive(Debug)]
pub struct WorktreeManager {
    /// Root of the main (non-worktree) repo.
    pub(crate) main_repo: PathBuf,
    /// Directory under which worktrees are created.
    pub(crate) worktree_base: PathBuf,
    /// Files copied from the main repo into each new worktree.
    pub(crate) copy_files: Vec<String>,
    /// Directories symlinked from the main repo (read-only caches).
    pub(crate) symlink_dirs: Vec<String>,
    /// Maximum concurrent worktrees (hard limit).
    pub(crate) max_concurrent: usize,
    /// Command timeout in seconds.
    pub(crate) timeout_secs: u64,
}

impl WorktreeManager {
    /// Create a `WorktreeManager` from config fields.
    pub fn new(
        main_repo: PathBuf,
        base_dir: &str,
        copy_files: Vec<String>,
        symlink_dirs: Vec<String>,
        max_concurrent: usize,
        timeout_secs: u64,
    ) -> Self {
        let worktree_base = main_repo.join(base_dir);
        Self {
            main_repo,
            worktree_base,
            copy_files,
            symlink_dirs,
            max_concurrent,
            timeout_secs,
        }
    }

    /// Creates a new worktree for `branch` based on `base_ref` (default: HEAD).
    ///
    /// # Steps
    /// 1. Enforce `max_concurrent` limit.
    /// 2. `git worktree add -b {branch} {path} {base_ref}`
    /// 3. Hard-copy files from `copy_files` (avoids race conditions).
    /// 4. Symlink read-only caches from `symlink_dirs`.
    /// 5. Write `.orka-worktree-meta.json`.
    ///
    /// # Errors
    /// Returns [`GitError`] on policy violations, I/O errors, or git failures.
    pub async fn create(
        &self,
        branch: &str,
        base_ref: Option<&str>,
        agent_id: Option<String>,
    ) -> Result<WorktreeInfo, GitError> {
        // Enforce max_concurrent
        let existing = self.list().await?;
        if existing.len() >= self.max_concurrent {
            return Err(GitError::Policy(format!(
                "maximum concurrent worktrees reached ({}/{})",
                existing.len(),
                self.max_concurrent
            )));
        }

        let name = branch_to_slug(branch);
        let wt_path = self.worktree_base.join(&name);

        if wt_path.exists() {
            return Err(GitError::Policy(format!(
                "worktree directory '{}' already exists",
                wt_path.display()
            )));
        }

        // Create the worktree_base directory if needed
        std::fs::create_dir_all(&self.worktree_base)?;

        // git worktree add
        let wt_path_str = wt_path.display().to_string();
        let base = base_ref.unwrap_or("HEAD");
        let out = run_git(
            &self.main_repo,
            &["worktree", "add", "-b", branch, &wt_path_str, base],
            self.timeout_secs,
        )
        .await?
        .into_result()
        .map_err(|e| GitError::Command(format!("worktree add failed: {e}")))?;

        info!(branch, path = %wt_path.display(), "created worktree");
        tracing::debug!(stdout = %out.stdout.trim(), "git worktree add output");

        // Copy files (hard copy, not symlink — avoids race conditions if source
        // changes while agent is running)
        for file in &self.copy_files {
            let src = self.main_repo.join(file);
            let dst = wt_path.join(file);
            if src.exists() {
                if let Some(parent) = dst.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&src, &dst)?;
            }
        }

        // Symlink read-only cache directories
        for dir in &self.symlink_dirs {
            let src = self.main_repo.join(dir);
            let dst = wt_path.join(dir);
            if src.exists() {
                // Remove existing destination first (e.g., empty dir created by git)
                if dst.exists() || dst.symlink_metadata().is_ok() {
                    if dst.is_dir() && !dst.is_symlink() {
                        let _ = std::fs::remove_dir_all(&dst);
                    } else {
                        let _ = std::fs::remove_file(&dst);
                    }
                }
                #[cfg(unix)]
                std::os::unix::fs::symlink(&src, &dst)?;
                #[cfg(not(unix))]
                warn!(src = %src.display(), dst = %dst.display(), "symlink_dirs unsupported on non-Unix; skipping");
            }
        }

        // Write metadata
        let meta = WorktreeMeta {
            name: name.clone(),
            branch: branch.to_string(),
            created_at: Utc::now(),
            agent_id: agent_id.clone(),
        };
        let meta_path = wt_path.join(".orka-worktree-meta.json");
        let json =
            serde_json::to_string_pretty(&meta).map_err(|e| GitError::Command(e.to_string()))?;
        std::fs::write(&meta_path, json)?;

        // Get HEAD sha for the new worktree
        let head_sha = run_git(
            &wt_path,
            &["rev-parse", "--short", "HEAD"],
            self.timeout_secs,
        )
        .await
        .ok()
        .map(|o| o.stdout.trim().to_string())
        .unwrap_or_default();

        Ok(WorktreeInfo {
            name,
            path: wt_path,
            branch: branch.to_string(),
            head_sha,
            created_at: Some(meta.created_at),
            agent_id,
        })
    }

    /// Lists all active worktrees (excluding the main repo).
    ///
    /// Uses `git worktree list --porcelain` for structured output.
    pub async fn list(&self) -> Result<Vec<WorktreeInfo>, GitError> {
        let out = run_git(
            &self.main_repo,
            &["worktree", "list", "--porcelain"],
            self.timeout_secs,
        )
        .await?
        .into_result()?;

        let all = parse_worktree_list(&out.stdout);
        // Filter out the main worktree
        let main_canonical = self
            .main_repo
            .canonicalize()
            .unwrap_or_else(|_| self.main_repo.clone());

        let mut result = Vec::new();
        for entry in all {
            let canonical = entry
                .path
                .canonicalize()
                .unwrap_or_else(|_| entry.path.clone());
            if canonical == main_canonical {
                continue;
            }
            // Load metadata if present
            let meta_path = entry.path.join(".orka-worktree-meta.json");
            let (created_at, agent_id) = if let Ok(raw) = std::fs::read_to_string(&meta_path) {
                let meta: Option<WorktreeMeta> = serde_json::from_str(&raw).ok();
                (
                    meta.as_ref().map(|m| m.created_at),
                    meta.and_then(|m| m.agent_id),
                )
            } else {
                (None, None)
            };
            result.push(WorktreeInfo {
                name: entry.name,
                path: entry.path,
                branch: entry.branch,
                head_sha: entry.head_sha,
                created_at,
                agent_id,
            });
        }
        Ok(result)
    }

    /// Removes a worktree by name.
    ///
    /// Runs `git worktree remove` followed by `git worktree prune`.
    /// Use `force = true` to remove even if the worktree has uncommitted
    /// changes.
    pub async fn remove(&self, name: &str, force: bool) -> Result<(), GitError> {
        let wt_path = self.worktree_base.join(name);
        if !wt_path.exists() {
            return Err(GitError::InvalidArg(format!(
                "worktree '{name}' not found at '{}'",
                wt_path.display()
            )));
        }

        let wt_path_str = wt_path.display().to_string();
        let args: Vec<&str> = if force {
            vec!["worktree", "remove", "--force", &wt_path_str]
        } else {
            vec!["worktree", "remove", &wt_path_str]
        };

        run_git(&self.main_repo, &args, self.timeout_secs)
            .await?
            .into_result()
            .map_err(|e| GitError::Command(format!("worktree remove failed: {e}")))?;

        // Prune stale worktree entries from .git/worktrees/
        let _ = run_git(&self.main_repo, &["worktree", "prune"], self.timeout_secs).await;

        info!(name, "removed worktree");
        Ok(())
    }
}

// ── Internal helpers ─────────────────────────────────────────────────────────

struct ParsedWorktree {
    path: PathBuf,
    head_sha: String,
    branch: String,
    name: String,
}

fn parse_worktree_list(output: &str) -> Vec<ParsedWorktree> {
    let mut result = Vec::new();
    let mut path = None::<PathBuf>;
    let mut head = String::new();
    let mut branch = String::new();

    for line in output.lines() {
        if let Some(wt_path) = line.strip_prefix("worktree ") {
            // Flush previous
            if let Some(p) = path.take() {
                let name = branch_to_slug(&branch);
                result.push(ParsedWorktree {
                    path: p,
                    head_sha: head.get(..7).map_or_else(|| head.clone(), String::from),
                    branch: branch.clone(),
                    name,
                });
            }
            path = Some(PathBuf::from(wt_path));
            head.clear();
            branch.clear();
        } else if let Some(sha) = line.strip_prefix("HEAD ") {
            sha.clone_into(&mut head);
        } else if let Some(b) = line.strip_prefix("branch ") {
            branch = b.trim().trim_start_matches("refs/heads/").to_string();
        } else if line == "detached" {
            branch = "(HEAD detached)".to_string();
        }
    }
    // Flush last
    if let Some(p) = path {
        let name = branch_to_slug(&branch);
        result.push(ParsedWorktree {
            path: p,
            head_sha: head.get(..7).map_or_else(|| head.clone(), String::from),
            branch,
            name,
        });
    }
    result
}

/// Converts a branch name to a safe filesystem slug.
/// `feat/my-feature` → `feat-my-feature`
fn branch_to_slug(branch: &str) -> String {
    branch
        .trim_start_matches("refs/heads/")
        .chars()
        .map(|c| if c == '/' || c == '_' { '-' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_conversion() {
        assert_eq!(branch_to_slug("feat/add-auth"), "feat-add-auth");
        assert_eq!(branch_to_slug("main"), "main");
        assert_eq!(branch_to_slug("fix/bug_42"), "fix-bug-42");
    }

    #[test]
    fn parse_simple_worktree_list() {
        let output = "\
worktree /home/user/project
HEAD abc1234567890abcdef
branch refs/heads/main

worktree /home/user/project/.orka-worktrees/feat-x
HEAD def5678901234abcdef
branch refs/heads/feat/x

";
        let parsed = parse_worktree_list(output);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1].branch, "feat/x");
        assert_eq!(parsed[1].head_sha, "def5678");
    }
}
