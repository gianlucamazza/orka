//! gix-based repository operations for read-only git skills.
//!
//! Uses `tokio::task::spawn_blocking` since the `gix` API is synchronous.
//! Write operations and CLI-dependent operations (push, signing,
//! worktree management) go through [`crate::cli::run_git`].

use std::path::{Path, PathBuf};

use gix::bstr::ByteSlice as _;

use crate::error::GitError;

/// Parsed information about a single branch.
#[derive(Debug, Clone)]
pub struct BranchInfo {
    /// Short branch name (e.g. `"main"`, `"feat/x"`).
    pub name: String,
    /// Remote tracking name (e.g. `"origin/main"`), if any.
    pub upstream: Option<String>,
    /// Whether this is the currently checked-out branch.
    pub is_current: bool,
    /// Whether the ref is a remote-tracking branch.
    pub is_remote: bool,
}

/// Summary entry from the commit log.
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// Short (7-char) commit SHA.
    pub sha: String,
    /// Full commit SHA.
    pub sha_full: String,
    /// Author display name.
    pub author_name: String,
    /// Author email.
    pub author_email: String,
    /// Commit date (ISO 8601 / RFC 3339).
    pub date: String,
    /// First line of the commit message.
    pub subject: String,
    /// Remainder of the commit message (after the blank line).
    pub body: String,
}

/// Opens a gix repository, discovering upwards from `path`.
/// Returns the resolved work-dir path.
///
/// Runs blocking I/O on a `spawn_blocking` thread.
///
/// # Errors
/// Returns [`GitError::Discover`] if no repository is found.
pub async fn discover_path(path: PathBuf) -> Result<PathBuf, GitError> {
    tokio::task::spawn_blocking(move || {
        let repo = gix::discover(&path).map_err(|e| GitError::Discover {
            path: path.display().to_string(),
            source: Box::new(e),
        })?;
        Ok(repo
            .workdir()
            .map_or_else(|| repo.git_dir().to_path_buf(), Path::to_path_buf))
    })
    .await
    .map_err(|e| GitError::Gix(e.to_string()))?
}

/// Lists all local and remote branches for the repo at `repo_path`.
///
/// Returns `(branches, current_branch_name)`.
///
/// Runs blocking I/O on a `spawn_blocking` thread.
pub async fn list_branches(repo_path: PathBuf) -> Result<(Vec<BranchInfo>, String), GitError> {
    tokio::task::spawn_blocking(move || {
        let repo = gix::discover(&repo_path).map_err(|e| GitError::Discover {
            path: repo_path.display().to_string(),
            source: Box::new(e),
        })?;

        // Current HEAD branch (symbolic ref target).
        let current = repo
            .head()
            .ok()
            .and_then(|h| {
                h.referent_name()
                    .map(|n| n.as_bstr().to_str_lossy().into_owned())
            })
            .map(|s| s.strip_prefix("refs/heads/").map(String::from).unwrap_or(s))
            .unwrap_or_default();

        let refs = repo
            .references()
            .map_err(|e| GitError::Gix(e.to_string()))?;
        let mut branches = Vec::new();

        for r in refs.all().map_err(|e| GitError::Gix(e.to_string()))? {
            let r = r.map_err(|e| GitError::Gix(e.to_string()))?;
            let full_name = r.name().as_bstr().to_str_lossy().into_owned();

            let (name, is_remote) = if let Some(n) = full_name.strip_prefix("refs/heads/") {
                (n.to_string(), false)
            } else if let Some(n) = full_name.strip_prefix("refs/remotes/") {
                if n.ends_with("/HEAD") {
                    continue;
                }
                (n.to_string(), true)
            } else {
                continue;
            };

            let is_current = !is_remote && name == current;
            branches.push(BranchInfo {
                name,
                upstream: None,
                is_current,
                is_remote,
            });
        }

        branches.sort_by(|a, b| a.name.cmp(&b.name));
        Ok((branches, current))
    })
    .await
    .map_err(|e| GitError::Gix(e.to_string()))?
}

/// Walks the commit log from HEAD, returning up to `max_count` entries.
///
/// Optional filters: `author_filter` and `grep_filter` do substring matching.
/// `path_filter` is unsupported via gix (requires tree diff); use CLI
/// [`crate::cli::run_git`] with `git log -- <path>` for that.
///
/// Runs blocking I/O on a `spawn_blocking` thread.
pub async fn walk_log(
    repo_path: PathBuf,
    max_count: usize,
    path_filter: Option<String>,
    author_filter: Option<String>,
    grep_filter: Option<String>,
) -> Result<Vec<LogEntry>, GitError> {
    // Path-filtered log requires checking changed files per commit; fall back
    // to CLI immediately.
    if path_filter.is_some() {
        return Err(GitError::InvalidArg(
            "path-filtered log must go through CLI; use git_log with path parameter".to_string(),
        ));
    }

    tokio::task::spawn_blocking(move || {
        let repo = gix::discover(&repo_path).map_err(|e| GitError::Discover {
            path: repo_path.display().to_string(),
            source: Box::new(e),
        })?;

        let head_id = repo
            .head_id()
            .map_err(|e| GitError::Gix(e.to_string()))?
            .detach();

        let walk = repo
            .rev_walk([head_id])
            .all()
            .map_err(|e| GitError::Gix(e.to_string()))?;

        let mut entries = Vec::new();

        for info in walk {
            if entries.len() >= max_count {
                break;
            }
            let info = info.map_err(|e| GitError::Gix(e.to_string()))?;
            let obj = repo
                .find_object(info.id)
                .map_err(|e| GitError::Gix(e.to_string()))?;
            let commit = obj
                .try_into_commit()
                .map_err(|_| GitError::Gix("object is not a commit".to_string()))?;
            let decoded = commit.decode().map_err(|e| GitError::Gix(e.to_string()))?;

            // In gix-object 0.58, `CommitRef::author` is raw `&BStr`.
            // Call `.author()` method to get the parsed `SignatureRef`.
            let author_sig = decoded.author().map_err(|e| GitError::Gix(e.to_string()))?;
            let author_name = author_sig.name.to_str_lossy().into_owned();
            let author_email = author_sig.email.to_str_lossy().into_owned();
            let message = decoded.message.to_str_lossy().into_owned();
            let (subject, body) = split_message(&message);

            if let Some(ref af) = author_filter {
                let laf = af.to_lowercase();
                if !author_name.to_lowercase().contains(&laf)
                    && !author_email.to_lowercase().contains(&laf)
                {
                    continue;
                }
            }

            if grep_filter
                .as_deref()
                .is_some_and(|gf| !message.to_lowercase().contains(&gf.to_lowercase()))
            {
                continue;
            }

            let sha_full = format!("{}", info.id);
            let sha = sha_full
                .get(..7)
                .map_or_else(|| sha_full.clone(), String::from);

            let date = format_git_time(author_sig.seconds());

            entries.push(LogEntry {
                sha,
                sha_full,
                author_name,
                author_email,
                date,
                subject: subject.to_string(),
                body: body.to_string(),
            });
        }

        Ok(entries)
    })
    .await
    .map_err(|e| GitError::Gix(e.to_string()))?
}

fn split_message(msg: &str) -> (&str, &str) {
    let trimmed = msg.trim_end_matches('\n');
    if let Some(pos) = trimmed.find("\n\n") {
        (&trimmed[..pos], trimmed[pos + 2..].trim())
    } else {
        (trimmed, "")
    }
}

fn format_git_time(seconds: i64) -> String {
    use chrono::{TimeZone, Utc};
    Utc.timestamp_opt(seconds, 0)
        .single()
        .map_or_else(|| seconds.to_string(), |dt| dt.to_rfc3339())
}
