//! Search skills: `git_blame`, `git_grep`.

use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema, traits::Skill};

use crate::{
    cli::{resolve_work_dir, run_git},
    guard::GitGuard,
};

// ── git_blame
// ─────────────────────────────────────────────────────────────────

/// Shows per-line commit attribution for a file.
pub struct GitBlameSkill {
    guard: Arc<GitGuard>,
}

impl GitBlameSkill {
    /// Create from the shared guard.
    pub fn new(guard: Arc<GitGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for GitBlameSkill {
    fn name(&self) -> &'static str {
        "git_blame"
    }

    fn category(&self) -> &'static str {
        "git"
    }

    fn description(&self) -> &'static str {
        "Show which commit last modified each line of a file, with author and date. \
         Optionally limit to a line range."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "required": ["file_path"],
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file to blame (relative to repository root)"
                },
                "start_line": {
                    "type": "integer",
                    "description": "First line of the range (1-based)"
                },
                "end_line": {
                    "type": "integer",
                    "description": "Last line of the range (1-based)"
                },
                "path": {
                    "type": "string",
                    "description": "Repository path"
                }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let file_path = input
            .args
            .get("file_path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| orka_core::Error::SkillCategorized {
                message: "missing 'file_path' argument".to_string(),
                category: orka_core::ErrorCategory::Input,
            })?;

        let cwd_str = input.args.get("path").and_then(serde_json::Value::as_str);
        let worktree_cwd = input
            .context
            .as_ref()
            .and_then(|c| c.worktree_cwd.as_deref());
        let user_cwd = input.context.as_ref().and_then(|c| c.user_cwd.as_deref());
        let work_dir = resolve_work_dir(cwd_str.or(worktree_cwd).or(user_cwd))
            .map_err(orka_core::Error::from)?;

        let start_line = input
            .args
            .get("start_line")
            .and_then(serde_json::Value::as_u64);
        let end_line = input
            .args
            .get("end_line")
            .and_then(serde_json::Value::as_u64);

        // porcelain format: machine-readable per-commit blocks
        let mut args = vec!["blame", "--porcelain"];

        let line_range;
        match (start_line, end_line) {
            (Some(s), Some(e)) => {
                line_range = format!("-L {s},{e}");
                args.push(&line_range);
            }
            (Some(s), None) => {
                line_range = format!("-L {s}");
                args.push(&line_range);
            }
            _ => {}
        }

        args.push("--");
        args.push(file_path);

        let out = run_git(&work_dir, &args, self.guard.command_timeout_secs)
            .await
            .map_err(orka_core::Error::from)?
            .into_result()
            .map_err(orka_core::Error::from)?;

        let lines = parse_blame_porcelain(&out.stdout);

        Ok(SkillOutput::new(serde_json::json!({
            "file": file_path,
            "lines": lines,
            "count": lines.len(),
        })))
    }
}

/// Parse `git blame --porcelain` output into structured line entries.
fn parse_blame_porcelain(output: &str) -> Vec<serde_json::Value> {
    let mut lines = Vec::new();
    let mut iter = output.lines().peekable();

    while let Some(header) = iter.next() {
        // Header: "<sha> <orig_line> <final_line> [<group_count>]"
        let parts: Vec<&str> = header.split_whitespace().collect();
        if parts.len() < 3 || parts[0].len() < 40 {
            continue;
        }
        let sha = parts[0];
        let line_no: u64 = parts[2].parse().unwrap_or(0);

        let mut author = "";
        let mut author_mail = "";
        let mut author_time = "";
        let mut summary = "";
        let mut content = "";

        // Read key-value pairs until the content line (tab-prefixed)
        for kv in iter.by_ref() {
            if let Some(c) = kv.strip_prefix('\t') {
                content = c;
                break;
            }
            if let Some(v) = kv.strip_prefix("author ") {
                author = v;
            } else if let Some(v) = kv.strip_prefix("author-mail ") {
                author_mail = v.trim_matches(|c| c == '<' || c == '>');
            } else if let Some(v) = kv.strip_prefix("author-time ") {
                author_time = v;
            } else if let Some(v) = kv.strip_prefix("summary ") {
                summary = v;
            }
        }

        lines.push(serde_json::json!({
            "line_no": line_no,
            "sha": &sha[..8],
            "sha_full": sha,
            "author": author,
            "email": author_mail,
            "timestamp": author_time,
            "summary": summary,
            "content": content,
        }));
    }

    lines
}

// ── git_grep
// ──────────────────────────────────────────────────────────────────

/// Searches for a pattern across tracked files.
pub struct GitGrepSkill {
    guard: Arc<GitGuard>,
}

impl GitGrepSkill {
    /// Create from the shared guard.
    pub fn new(guard: Arc<GitGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for GitGrepSkill {
    fn name(&self) -> &'static str {
        "git_grep"
    }

    fn category(&self) -> &'static str {
        "git"
    }

    fn description(&self) -> &'static str {
        "Search for a pattern in tracked files using git grep. \
         Faster than filesystem grep because it searches only indexed files."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Pattern to search for (extended regex)"
                },
                "ignore_case": {
                    "type": "boolean",
                    "description": "Case-insensitive search",
                    "default": false
                },
                "path_pattern": {
                    "type": "string",
                    "description": "Glob pattern to limit search to matching file paths (e.g. '*.rs')"
                },
                "context_lines": {
                    "type": "integer",
                    "description": "Lines of context around each match",
                    "default": 0
                },
                "max_matches": {
                    "type": "integer",
                    "description": "Stop after this many matches",
                    "default": 100
                },
                "path": {
                    "type": "string",
                    "description": "Repository path"
                }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let pattern = input
            .args
            .get("pattern")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| orka_core::Error::SkillCategorized {
                message: "missing 'pattern' argument".to_string(),
                category: orka_core::ErrorCategory::Input,
            })?;
        let ignore_case = input
            .args
            .get("ignore_case")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let path_pattern = input
            .args
            .get("path_pattern")
            .and_then(serde_json::Value::as_str);
        let context_lines = input
            .args
            .get("context_lines")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let max_matches = input
            .args
            .get("max_matches")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(100) as usize;

        let cwd_str = input.args.get("path").and_then(serde_json::Value::as_str);
        let worktree_cwd = input
            .context
            .as_ref()
            .and_then(|c| c.worktree_cwd.as_deref());
        let user_cwd = input.context.as_ref().and_then(|c| c.user_cwd.as_deref());
        let work_dir = resolve_work_dir(cwd_str.or(worktree_cwd).or(user_cwd))
            .map_err(orka_core::Error::from)?;

        let mut args = vec!["grep", "--extended-regexp", "-n", "--column"];
        if ignore_case {
            args.push("-i");
        }
        let ctx_arg;
        if context_lines > 0 {
            ctx_arg = format!("-C{context_lines}");
            args.push(&ctx_arg);
        }
        args.push(pattern);

        let pp_arg;
        if let Some(pp) = path_pattern {
            args.push("--");
            pp_arg = format!(":(glob){pp}");
            args.push(&pp_arg);
        }

        let out = run_git(&work_dir, &args, self.guard.command_timeout_secs)
            .await
            .map_err(orka_core::Error::from)?;

        // grep exits with 1 when no matches — treat as empty result, not error
        let stdout = if out.exit_code == 0 || out.exit_code == 1 {
            &out.stdout
        } else {
            out.into_result().map_err(orka_core::Error::from)?;
            unreachable!()
        };

        let matches: Vec<serde_json::Value> = stdout
            .lines()
            .filter(|l| !l.is_empty())
            .take(max_matches)
            .filter_map(|l| {
                // Format: file:line_no:column:content
                let mut parts = l.splitn(4, ':');
                let file = parts.next()?;
                let line: u64 = parts.next()?.parse().ok()?;
                let _col: u64 = parts.next()?.parse().ok()?;
                let content = parts.next().unwrap_or("");
                Some(serde_json::json!({
                    "file": file,
                    "line": line,
                    "content": content,
                }))
            })
            .collect();

        let truncated = matches.len() == max_matches && stdout.lines().count() > max_matches;

        Ok(SkillOutput::new(serde_json::json!({
            "pattern": pattern,
            "matches": matches,
            "count": matches.len(),
            "truncated": truncated,
        })))
    }
}
