use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use orka_core::config::OsConfig;
use orka_core::traits::Skill;
use orka_core::{Error, Result, SkillInput, SkillOutput, SkillSchema};
use tracing::debug;

use crate::config::PermissionLevel;
use crate::guard::PermissionGuard;

// ── fs_read ──

pub struct FsReadSkill {
    guard: Arc<PermissionGuard>,
    max_output_bytes: usize,
}

impl FsReadSkill {
    pub fn new(guard: Arc<PermissionGuard>, config: &OsConfig) -> Self {
        Self {
            guard,
            max_output_bytes: config.max_output_bytes,
        }
    }
}

#[async_trait]
impl Skill for FsReadSkill {
    fn name(&self) -> &str {
        "fs_read"
    }

    fn description(&self) -> &str {
        "Read a file's contents. Supports text (UTF-8) and binary (base64) encoding."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to read" },
                    "offset": { "type": "integer", "description": "Byte offset to start reading from", "default": 0 },
                    "limit": { "type": "integer", "description": "Maximum bytes to read" },
                    "encoding": { "type": "string", "enum": ["utf8", "base64"], "default": "utf8" }
                },
                "required": ["path"]
            }),
        }
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let path_str = input
            .args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Skill("missing 'path' argument".into()))?;
        let encoding = input
            .args
            .get("encoding")
            .and_then(|v| v.as_str())
            .unwrap_or("utf8");
        let offset = input
            .args
            .get("offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let limit = input
            .args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);

        let path = Path::new(path_str);
        let canonical = self.guard.check_path(path)?;

        let metadata = tokio::fs::metadata(&canonical)
            .await
            .map_err(|e| Error::Skill(format!("cannot read '{}': {}", canonical.display(), e)))?;
        self.guard.check_file_size(metadata.len())?;

        let bytes = tokio::fs::read(&canonical).await.map_err(|e| {
            Error::Skill(format!("failed to read '{}': {}", canonical.display(), e))
        })?;

        let end = limit
            .map(|l| (offset + l).min(bytes.len()))
            .unwrap_or(bytes.len());
        let slice = &bytes[offset.min(bytes.len())..end.min(bytes.len())];

        let (content, actual_encoding) = if encoding == "base64" {
            (base64_encode(slice), "base64")
        } else {
            match std::str::from_utf8(slice) {
                Ok(s) => {
                    let truncated = if s.len() > self.max_output_bytes {
                        &s[..self.max_output_bytes]
                    } else {
                        s
                    };
                    (truncated.to_string(), "utf8")
                }
                Err(_) => (base64_encode(slice), "base64"),
            }
        };

        Ok(SkillOutput {
            data: serde_json::json!({
                "content": content,
                "encoding": actual_encoding,
                "size_bytes": metadata.len(),
                "path": canonical.to_string_lossy(),
            }),
        })
    }
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

// ── fs_list ──

pub struct FsListSkill {
    guard: Arc<PermissionGuard>,
    max_list_entries: usize,
}

impl FsListSkill {
    pub fn new(guard: Arc<PermissionGuard>, config: &OsConfig) -> Self {
        Self {
            guard,
            max_list_entries: config.max_list_entries,
        }
    }
}

#[async_trait]
impl Skill for FsListSkill {
    fn name(&self) -> &str {
        "fs_list"
    }

    fn description(&self) -> &str {
        "List files and directories at a given path."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory to list" },
                    "recursive": { "type": "boolean", "default": false },
                    "show_hidden": { "type": "boolean", "default": false },
                    "pattern": { "type": "string", "description": "Glob pattern to filter results" }
                },
                "required": ["path"]
            }),
        }
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let path_str = input
            .args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Skill("missing 'path' argument".into()))?;
        let recursive = input
            .args
            .get("recursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let show_hidden = input
            .args
            .get("show_hidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let pattern = input.args.get("pattern").and_then(|v| v.as_str());

        let path = Path::new(path_str);
        let canonical = self.guard.check_path(path)?;

        let mut entries = Vec::new();
        list_dir(
            &canonical,
            recursive,
            show_hidden,
            pattern,
            &mut entries,
            self.max_list_entries,
        )
        .await?;

        Ok(SkillOutput {
            data: serde_json::json!({
                "entries": entries,
                "total": entries.len(),
                "path": canonical.to_string_lossy(),
            }),
        })
    }
}

async fn list_dir(
    path: &Path,
    recursive: bool,
    show_hidden: bool,
    pattern: Option<&str>,
    entries: &mut Vec<serde_json::Value>,
    max: usize,
) -> Result<()> {
    let mut read_dir = tokio::fs::read_dir(path)
        .await
        .map_err(|e| Error::Skill(format!("cannot list '{}': {}", path.display(), e)))?;

    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|e| Error::Skill(format!("error reading directory entry: {}", e)))?
    {
        if entries.len() >= max {
            break;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        if !show_hidden && name.starts_with('.') {
            continue;
        }

        if let Some(pat) = pattern
            && !glob::Pattern::new(pat)
                .map(|p| p.matches(&name))
                .unwrap_or(true)
        {
            continue;
        }

        let metadata = entry.metadata().await.ok();
        let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);

        entries.push(serde_json::json!({
            "name": name,
            "path": entry.path().to_string_lossy(),
            "is_dir": is_dir,
            "size": metadata.as_ref().map(|m| m.len()).unwrap_or(0),
            "modified": metadata.as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
            "permissions": metadata.as_ref().map(|m| format!("{:o}", m.permissions().mode() & 0o7777)),
        }));

        if recursive && is_dir && entries.len() < max {
            Box::pin(list_dir(
                &entry.path(),
                true,
                show_hidden,
                pattern,
                entries,
                max,
            ))
            .await?;
        }
    }
    Ok(())
}

// ── fs_info ──

pub struct FsInfoSkill {
    guard: Arc<PermissionGuard>,
}

impl FsInfoSkill {
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for FsInfoSkill {
    fn name(&self) -> &str {
        "fs_info"
    }

    fn description(&self) -> &str {
        "Get detailed metadata for a file or directory (size, type, permissions, timestamps)."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to inspect" }
                },
                "required": ["path"]
            }),
        }
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let path_str = input
            .args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Skill("missing 'path' argument".into()))?;

        let path = Path::new(path_str);
        let canonical = self.guard.check_path(path)?;

        let metadata = tokio::fs::metadata(&canonical)
            .await
            .map_err(|e| Error::Skill(format!("cannot stat '{}': {}", canonical.display(), e)))?;
        let symlink_meta = tokio::fs::symlink_metadata(&canonical).await.ok();

        let file_type = if metadata.is_dir() {
            "directory"
        } else if metadata.is_file() {
            "file"
        } else if metadata.is_symlink() {
            "symlink"
        } else {
            "other"
        };

        let is_symlink = symlink_meta
            .as_ref()
            .map(|m| m.is_symlink())
            .unwrap_or(false);

        Ok(SkillOutput {
            data: serde_json::json!({
                "path": canonical.to_string_lossy(),
                "size_bytes": metadata.len(),
                "file_type": file_type,
                "is_symlink": is_symlink,
                "permissions": format!("{:o}", metadata.permissions().mode() & 0o7777),
                "modified": metadata.modified().ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs()),
                "accessed": metadata.accessed().ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs()),
                "created": metadata.created().ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs()),
            }),
        })
    }
}

// ── fs_search ──

pub struct FsSearchSkill {
    guard: Arc<PermissionGuard>,
    max_list_entries: usize,
}

impl FsSearchSkill {
    pub fn new(guard: Arc<PermissionGuard>, config: &OsConfig) -> Self {
        Self {
            guard,
            max_list_entries: config.max_list_entries,
        }
    }
}

#[async_trait]
impl Skill for FsSearchSkill {
    fn name(&self) -> &str {
        "fs_search"
    }

    fn description(&self) -> &str {
        "Search for files by name (glob), or search file contents for a pattern."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Root directory to search" },
                    "pattern": { "type": "string", "description": "Search pattern" },
                    "by": {
                        "type": "string",
                        "enum": ["name", "content", "glob"],
                        "default": "glob",
                        "description": "Search mode"
                    },
                    "max_results": { "type": "integer", "default": 50, "description": "Max results" }
                },
                "required": ["path", "pattern"]
            }),
        }
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let path_str = input
            .args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Skill("missing 'path' argument".into()))?;
        let pattern = input
            .args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Skill("missing 'pattern' argument".into()))?;
        let by = input
            .args
            .get("by")
            .and_then(|v| v.as_str())
            .unwrap_or("glob");
        let max_results = input
            .args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as usize;

        let path = Path::new(path_str);
        let canonical = self.guard.check_path(path)?;

        let max = max_results.min(self.max_list_entries);

        match by {
            "glob" => {
                let glob_pattern = format!("{}/{}", canonical.display(), pattern);
                let mut results = Vec::new();
                for entry in glob::glob(&glob_pattern)
                    .map_err(|e| Error::Skill(format!("invalid glob pattern: {}", e)))?
                {
                    if results.len() >= max {
                        break;
                    }
                    if let Ok(p) = entry {
                        results.push(serde_json::json!(p.to_string_lossy()));
                    }
                }
                Ok(SkillOutput {
                    data: serde_json::json!({
                        "matches": results,
                        "count": results.len(),
                        "search_type": "glob",
                    }),
                })
            }
            "name" => {
                let mut results = Vec::new();
                search_by_name(&canonical, pattern, &mut results, max).await?;
                Ok(SkillOutput {
                    data: serde_json::json!({
                        "matches": results,
                        "count": results.len(),
                        "search_type": "name",
                    }),
                })
            }
            "content" => {
                let mut results = Vec::new();
                search_by_content(&canonical, pattern, &mut results, max).await?;
                Ok(SkillOutput {
                    data: serde_json::json!({
                        "matches": results,
                        "count": results.len(),
                        "search_type": "content",
                    }),
                })
            }
            _ => Err(Error::Skill(format!("unknown search mode: {}", by))),
        }
    }
}

async fn search_by_name(
    dir: &Path,
    pattern: &str,
    results: &mut Vec<serde_json::Value>,
    max: usize,
) -> Result<()> {
    let mut read_dir = tokio::fs::read_dir(dir)
        .await
        .map_err(|e| Error::Skill(format!("cannot read '{}': {}", dir.display(), e)))?;

    let pat_lower = pattern.to_lowercase();
    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|e| Error::Skill(format!("error reading entry: {}", e)))?
    {
        if results.len() >= max {
            break;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.to_lowercase().contains(&pat_lower) {
            results.push(serde_json::json!(entry.path().to_string_lossy()));
        }
        if entry.metadata().await.map(|m| m.is_dir()).unwrap_or(false) && results.len() < max {
            Box::pin(search_by_name(&entry.path(), pattern, results, max)).await?;
        }
    }
    Ok(())
}

async fn search_by_content(
    dir: &Path,
    pattern: &str,
    results: &mut Vec<serde_json::Value>,
    max: usize,
) -> Result<()> {
    let mut read_dir = tokio::fs::read_dir(dir)
        .await
        .map_err(|e| Error::Skill(format!("cannot read '{}': {}", dir.display(), e)))?;

    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|e| Error::Skill(format!("error reading entry: {}", e)))?
    {
        if results.len() >= max {
            break;
        }
        let metadata = entry.metadata().await.ok();
        let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let is_file = metadata.as_ref().map(|m| m.is_file()).unwrap_or(false);

        if is_file {
            // Skip large files
            let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
            if size > 1_000_000 {
                continue;
            }
            if let Ok(content) = tokio::fs::read_to_string(entry.path()).await {
                let matches: Vec<(usize, &str)> = content
                    .lines()
                    .enumerate()
                    .filter(|(_, line)| line.contains(pattern))
                    .take(5)
                    .collect();
                if !matches.is_empty() {
                    results.push(serde_json::json!({
                        "file": entry.path().to_string_lossy(),
                        "matches": matches.iter().map(|(n, line)| {
                            serde_json::json!({ "line": n + 1, "text": line.chars().take(200).collect::<String>() })
                        }).collect::<Vec<_>>(),
                    }));
                }
            }
        } else if is_dir && results.len() < max {
            Box::pin(search_by_content(&entry.path(), pattern, results, max)).await?;
        }
    }
    Ok(())
}

// ── fs_write ──

pub struct FsWriteSkill {
    guard: Arc<PermissionGuard>,
}

impl FsWriteSkill {
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for FsWriteSkill {
    fn name(&self) -> &str {
        "fs_write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Supports write (overwrite) and append modes."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to write" },
                    "content": { "type": "string", "description": "Content to write" },
                    "mode": { "type": "string", "enum": ["write", "append"], "default": "write" },
                    "create_dirs": { "type": "boolean", "default": false, "description": "Create parent directories" },
                    "encoding": { "type": "string", "enum": ["utf8", "base64"], "default": "utf8" }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Write)?;

        let path_str = input
            .args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Skill("missing 'path' argument".into()))?;
        let content = input
            .args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Skill("missing 'content' argument".into()))?;
        let mode = input
            .args
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("write");
        let create_dirs = input
            .args
            .get("create_dirs")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        self.guard.check_file_size(content.len() as u64)?;

        let path = Path::new(path_str);
        let canonical = self.guard.check_write_path(path)?;

        if create_dirs && let Some(parent) = canonical.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::Skill(format!("cannot create directories: {}", e)))?;
        }

        let bytes_written = content.len();
        match mode {
            "append" => {
                use tokio::io::AsyncWriteExt;
                let mut file = tokio::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&canonical)
                    .await
                    .map_err(|e| {
                        Error::Skill(format!("cannot open '{}': {}", canonical.display(), e))
                    })?;
                file.write_all(content.as_bytes())
                    .await
                    .map_err(|e| Error::Skill(format!("write failed: {}", e)))?;
            }
            _ => {
                tokio::fs::write(&canonical, content.as_bytes())
                    .await
                    .map_err(|e| Error::Skill(format!("write failed: {}", e)))?;
            }
        }

        debug!(path = %canonical.display(), bytes = bytes_written, "fs_write complete");

        Ok(SkillOutput {
            data: serde_json::json!({
                "bytes_written": bytes_written,
                "path": canonical.to_string_lossy(),
            }),
        })
    }
}

// ── fs_watch ──

pub struct FsWatchSkill {
    guard: Arc<PermissionGuard>,
}

impl FsWatchSkill {
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for FsWatchSkill {
    fn name(&self) -> &str {
        "fs_watch"
    }

    fn description(&self) -> &str {
        "Watch a path for filesystem changes for a specified duration. Returns batch of events."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to watch" },
                    "duration_secs": { "type": "integer", "default": 5, "maximum": 30, "description": "How long to watch" },
                    "recursive": { "type": "boolean", "default": true }
                },
                "required": ["path"]
            }),
        }
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Execute)?;

        let path_str = input
            .args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Skill("missing 'path' argument".into()))?;
        let duration_secs = input
            .args
            .get("duration_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(5)
            .min(30);
        let recursive = input
            .args
            .get("recursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let path = Path::new(path_str);
        let canonical = self.guard.check_path(path)?;

        let (tx, mut rx) = tokio::sync::mpsc::channel(256);

        let mut watcher = notify::recommended_watcher(
            move |res: std::result::Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = tx.blocking_send(event);
                }
            },
        )
        .map_err(|e| Error::Skill(format!("cannot create watcher: {}", e)))?;

        use notify::Watcher;
        let mode = if recursive {
            notify::RecursiveMode::Recursive
        } else {
            notify::RecursiveMode::NonRecursive
        };
        watcher
            .watch(&canonical, mode)
            .map_err(|e| Error::Skill(format!("cannot watch '{}': {}", canonical.display(), e)))?;

        let mut events = Vec::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(duration_secs);

        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => break,
                ev = rx.recv() => {
                    match ev {
                        Some(event) => {
                            events.push(serde_json::json!({
                                "kind": format!("{:?}", event.kind),
                                "paths": event.paths.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>(),
                            }));
                        }
                        None => break,
                    }
                }
            }
        }

        Ok(SkillOutput {
            data: serde_json::json!({
                "events": events,
                "count": events.len(),
                "watched_path": canonical.to_string_lossy(),
                "duration_secs": duration_secs,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_guard(level: &str) -> Arc<PermissionGuard> {
        let config = OsConfig {
            permission_level: level.into(),
            allowed_paths: vec!["/tmp".into()],
            ..OsConfig::default()
        };
        Arc::new(PermissionGuard::new(&config))
    }

    fn test_os_config() -> OsConfig {
        OsConfig {
            allowed_paths: vec!["/tmp".into()],
            ..OsConfig::default()
        }
    }

    #[test]
    fn fs_read_schema_valid() {
        let skill = FsReadSkill::new(test_guard("read-only"), &test_os_config());
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "path");
    }

    #[tokio::test]
    async fn fs_read_happy_path() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "hello world").unwrap();

        let config = OsConfig {
            allowed_paths: vec![dir.path().to_string_lossy().to_string()],
            ..OsConfig::default()
        };
        let guard = Arc::new(PermissionGuard::new(&config));
        let skill = FsReadSkill::new(guard, &config);

        let mut args = HashMap::new();
        args.insert("path".into(), serde_json::json!(file.to_string_lossy()));
        let output = skill
            .execute(SkillInput {
                args,
                context: None,
            })
            .await
            .unwrap();
        assert_eq!(output.data["content"], "hello world");
        assert_eq!(output.data["encoding"], "utf8");
    }

    #[tokio::test]
    async fn fs_read_missing_path_errors() {
        let skill = FsReadSkill::new(test_guard("read-only"), &test_os_config());
        let input = SkillInput {
            args: HashMap::new(),
            context: None,
        };
        assert!(skill.execute(input).await.is_err());
    }

    #[tokio::test]
    async fn fs_list_happy_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "").unwrap();
        std::fs::write(dir.path().join("b.txt"), "").unwrap();

        let config = OsConfig {
            allowed_paths: vec![dir.path().to_string_lossy().to_string()],
            ..OsConfig::default()
        };
        let guard = Arc::new(PermissionGuard::new(&config));
        let skill = FsListSkill::new(guard, &config);

        let mut args = HashMap::new();
        args.insert(
            "path".into(),
            serde_json::json!(dir.path().to_string_lossy()),
        );
        let output = skill
            .execute(SkillInput {
                args,
                context: None,
            })
            .await
            .unwrap();
        assert_eq!(output.data["total"], 2);
    }

    #[tokio::test]
    async fn fs_write_requires_write_permission() {
        let guard = test_guard("read-only");
        let skill = FsWriteSkill::new(guard);

        let mut args = HashMap::new();
        args.insert("path".into(), serde_json::json!("/tmp/test_write.txt"));
        args.insert("content".into(), serde_json::json!("data"));
        let result = skill
            .execute(SkillInput {
                args,
                context: None,
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fs_write_happy_path() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("out.txt");

        let config = OsConfig {
            permission_level: "write".into(),
            allowed_paths: vec![dir.path().to_string_lossy().to_string()],
            ..OsConfig::default()
        };
        let guard = Arc::new(PermissionGuard::new(&config));
        let skill = FsWriteSkill::new(guard);

        let mut args = HashMap::new();
        args.insert("path".into(), serde_json::json!(file.to_string_lossy()));
        args.insert("content".into(), serde_json::json!("hello"));
        let output = skill
            .execute(SkillInput {
                args,
                context: None,
            })
            .await
            .unwrap();
        assert_eq!(output.data["bytes_written"], 5);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "hello");
    }

    #[test]
    fn fs_info_schema_valid() {
        let skill = FsInfoSkill::new(test_guard("read-only"));
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "path");
    }

    #[test]
    fn fs_search_schema_valid() {
        let skill = FsSearchSkill::new(test_guard("read-only"), &test_os_config());
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "path");
        assert_eq!(schema.parameters["required"][1], "pattern");
    }
}
