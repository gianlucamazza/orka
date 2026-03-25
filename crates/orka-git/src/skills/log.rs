//! `git_log` skill.
//!
//! Uses gix for walks without a path filter (fast, no process overhead).
//! Falls back to CLI for path-filtered log.

use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema, traits::Skill};

use crate::{
    cli::{resolve_work_dir, run_git},
    guard::GitGuard,
    repo,
};

/// Returns the commit log.
///
/// Uses gix for the common case (no path filter). Path-filtered log
/// uses `git log -- <path>` via CLI.
pub struct GitLogSkill {
    guard: Arc<GitGuard>,
}

impl GitLogSkill {
    /// Create from the shared guard.
    pub fn new(guard: Arc<GitGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for GitLogSkill {
    fn name(&self) -> &'static str {
        "git_log"
    }

    fn category(&self) -> &'static str {
        "git"
    }

    fn description(&self) -> &'static str {
        "Show the commit log. Supports filtering by author, date range, message pattern, \
         and file path. Returns structured commit entries."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Repository path (defaults to user's current working directory)"
                },
                "max_count": {
                    "type": "integer",
                    "description": "Maximum number of commits to return",
                    "default": 20
                },
                "author": {
                    "type": "string",
                    "description": "Filter by author name or email (substring match)"
                },
                "since": {
                    "type": "string",
                    "description": "Show commits after this date (e.g. '2024-01-01', '1 week ago')"
                },
                "until": {
                    "type": "string",
                    "description": "Show commits before this date"
                },
                "grep": {
                    "type": "string",
                    "description": "Filter commits whose message matches this pattern"
                },
                "file_path": {
                    "type": "string",
                    "description": "Only show commits that touched this file path"
                }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let cwd_str = input.args.get("path").and_then(serde_json::Value::as_str);
        let worktree_cwd = input
            .context
            .as_ref()
            .and_then(|c| c.worktree_cwd.as_deref());
        let user_cwd = input.context.as_ref().and_then(|c| c.user_cwd.as_deref());
        let work_dir = resolve_work_dir(cwd_str.or(worktree_cwd).or(user_cwd))
            .map_err(orka_core::Error::from)?;

        let max_count = input
            .args
            .get("max_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(20) as usize;
        let max_count = max_count.min(self.guard.max_log_entries);

        let author = input
            .args
            .get("author")
            .and_then(serde_json::Value::as_str)
            .map(String::from);
        let since = input
            .args
            .get("since")
            .and_then(serde_json::Value::as_str)
            .map(String::from);
        let until = input
            .args
            .get("until")
            .and_then(serde_json::Value::as_str)
            .map(String::from);
        let grep = input
            .args
            .get("grep")
            .and_then(serde_json::Value::as_str)
            .map(String::from);
        let file_path = input
            .args
            .get("file_path")
            .and_then(serde_json::Value::as_str)
            .map(String::from);

        let entries = if file_path.is_some() || since.is_some() || until.is_some() {
            // Date-range and path filters: use CLI
            log_via_cli(
                &work_dir,
                max_count,
                author.as_deref(),
                since.as_deref(),
                until.as_deref(),
                grep.as_deref(),
                file_path.as_deref(),
                self.guard.command_timeout_secs,
            )
            .await?
        } else {
            // Fast path: use gix rev-walk
            repo::walk_log(
                work_dir.clone(),
                max_count,
                None,
                author.clone(),
                grep.clone(),
            )
            .await
            .map_err(orka_core::Error::from)
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|e| {
                        serde_json::json!({
                            "sha": e.sha,
                            "sha_full": e.sha_full,
                            "author": e.author_name,
                            "email": e.author_email,
                            "date": e.date,
                            "subject": e.subject,
                            "body": e.body,
                        })
                    })
                    .collect::<Vec<_>>()
            })?
        };

        Ok(SkillOutput::new(serde_json::json!({
            "commits": entries,
            "count": entries.len(),
        })))
    }
}

#[allow(clippy::too_many_arguments)]
async fn log_via_cli(
    work_dir: &std::path::Path,
    max_count: usize,
    author: Option<&str>,
    since: Option<&str>,
    until: Option<&str>,
    grep: Option<&str>,
    file_path: Option<&str>,
    timeout_secs: u64,
) -> Result<Vec<serde_json::Value>> {
    let fmt = "%H\x1f%h\x1f%an\x1f%ae\x1f%aI\x1f%s\x1f%b\x1e";
    let count_str = max_count.to_string();
    let format_arg = format!("--format={fmt}");

    let mut args = vec!["log", &format_arg, "-n", &count_str];

    let author_arg;
    if let Some(a) = author {
        author_arg = format!("--author={a}");
        args.push(&author_arg);
    }
    let since_arg;
    if let Some(s) = since {
        since_arg = format!("--since={s}");
        args.push(&since_arg);
    }
    let until_arg;
    if let Some(u) = until {
        until_arg = format!("--until={u}");
        args.push(&until_arg);
    }
    let grep_arg;
    if let Some(g) = grep {
        grep_arg = format!("--grep={g}");
        args.push(&grep_arg);
    }
    if let Some(fp) = file_path {
        args.push("--");
        args.push(fp);
    }

    let out = run_git(work_dir, &args, timeout_secs)
        .await
        .map_err(orka_core::Error::from)?
        .into_result()
        .map_err(orka_core::Error::from)?;

    // Parse record-separated output
    let entries = out
        .stdout
        .split('\x1e')
        .filter(|r| !r.trim().is_empty())
        .map(|record| {
            let fields: Vec<&str> = record.trim().splitn(7, '\x1f').collect();
            serde_json::json!({
                "sha_full": fields.first().unwrap_or(&""),
                "sha": fields.get(1).unwrap_or(&""),
                "author": fields.get(2).unwrap_or(&""),
                "email": fields.get(3).unwrap_or(&""),
                "date": fields.get(4).unwrap_or(&""),
                "subject": fields.get(5).unwrap_or(&""),
                "body": fields.get(6).map_or("", |s| s.trim()),
            })
        })
        .collect();

    Ok(entries)
}
