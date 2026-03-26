use std::{os::unix::fs::PermissionsExt, path::Path, sync::Arc};

use async_trait::async_trait;
use orka_core::{
    Error, ErrorCategory, Result, SkillInput, SkillOutput, SkillSchema, traits::Skill,
};
use tracing::debug;

use crate::{config::PermissionLevel, guard::PermissionGuard};

fn missing_arg(arg: &str) -> Error {
    Error::SkillCategorized {
        message: format!("missing '{arg}' argument"),
        category: ErrorCategory::Input,
    }
}

fn fs_io_error(context: &str, e: &std::io::Error) -> Error {
    let category = match e.kind() {
        std::io::ErrorKind::PermissionDenied => ErrorCategory::Environmental,
        std::io::ErrorKind::NotFound => ErrorCategory::Input,
        _ => ErrorCategory::Unknown,
    };
    Error::SkillCategorized {
        message: format!("{context}: {e}"),
        category,
    }
}

// ── fs_read ──

/// Skill that reads the contents of a file within the allowed paths.
pub struct FsReadSkill {
    guard: Arc<PermissionGuard>,
    max_output_bytes: usize,
}

impl FsReadSkill {
    /// Create a new `fs_read` skill from config and a permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self {
            guard,
            max_output_bytes: 100 * 1024, // Default 100KB
        }
    }
}

#[async_trait]
impl Skill for FsReadSkill {
    fn name(&self) -> &'static str {
        "fs_read"
    }

    fn category(&self) -> &'static str {
        "filesystem"
    }

    fn description(&self) -> &'static str {
        "Read a file's contents. Supports text (UTF-8) and binary (base64) encoding."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to read" },
                "offset": { "type": "integer", "description": "Byte offset to start reading from", "default": 0 },
                "limit": { "type": "integer", "description": "Maximum bytes to read" },
                "encoding": { "type": "string", "enum": ["utf8", "base64"], "default": "utf8" }
            },
            "required": ["path"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let path_str = input
            .args
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| missing_arg("path"))?;
        let encoding = input
            .args
            .get("encoding")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("utf8");
        let offset = input
            .args
            .get("offset")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize;
        let limit = input
            .args
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .map(|n| n as usize);

        let path = input.resolve_path(path_str);
        let canonical = self.guard.check_path(&path)?;

        let metadata = tokio::fs::metadata(&canonical)
            .await
            .map_err(|e| fs_io_error(&format!("cannot read '{}'", canonical.display()), &e))?;
        self.guard.check_file_size(metadata.len())?;

        let bytes = tokio::fs::read(&canonical)
            .await
            .map_err(|e| fs_io_error(&format!("failed to read '{}'", canonical.display()), &e))?;

        let end = limit.map_or(bytes.len(), |l| (offset + l).min(bytes.len()));
        let slice = &bytes[offset.min(bytes.len())..end.min(bytes.len())];

        let (content, actual_encoding) = if encoding == "base64" {
            (base64_encode(slice), "base64")
        } else {
            match std::str::from_utf8(slice) {
                Ok(s) => {
                    let truncated = if s.len() > self.max_output_bytes {
                        &s[..s.floor_char_boundary(self.max_output_bytes)]
                    } else {
                        s
                    };
                    (truncated.to_string(), "utf8")
                }
                Err(_) => (base64_encode(slice), "base64"),
            }
        };

        Ok(SkillOutput::new(serde_json::json!({
            "content": content,
            "encoding": actual_encoding,
            "size_bytes": metadata.len(),
            "path": canonical.to_string_lossy(),
        })))
    }
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
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

/// Skill that lists directory contents, optionally recursively.
pub struct FsListSkill {
    guard: Arc<PermissionGuard>,
    max_list_entries: usize,
}

impl FsListSkill {
    /// Create a new `fs_list` skill from config and a permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self {
            guard,
            max_list_entries: 1000, // Default max entries
        }
    }
}

#[async_trait]
impl Skill for FsListSkill {
    fn name(&self) -> &'static str {
        "fs_list"
    }

    fn category(&self) -> &'static str {
        "filesystem"
    }

    fn description(&self) -> &'static str {
        "List files and directories at a given path."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Directory to list" },
                "recursive": { "type": "boolean", "default": false },
                "show_hidden": { "type": "boolean", "default": false },
                "pattern": { "type": "string", "description": "Glob pattern to filter results" }
            },
            "required": ["path"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let path_str = input
            .args
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| missing_arg("path"))?;
        let recursive = input
            .args
            .get("recursive")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let show_hidden = input
            .args
            .get("show_hidden")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let pattern = input
            .args
            .get("pattern")
            .and_then(serde_json::Value::as_str);

        let path = input.resolve_path(path_str);
        let canonical = self.guard.check_path(&path)?;

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

        Ok(SkillOutput::new(serde_json::json!({
            "entries": entries,
            "total": entries.len(),
            "path": canonical.to_string_lossy(),
        })))
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
        .map_err(|e| fs_io_error(&format!("cannot list '{}'", path.display()), &e))?;

    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|e| Error::SkillCategorized {
            message: format!("error reading directory entry: {e}"),
            category: ErrorCategory::Unknown,
        })?
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
        let is_dir = metadata.as_ref().is_some_and(std::fs::Metadata::is_dir);

        entries.push(serde_json::json!({
            "name": name,
            "path": entry.path().to_string_lossy(),
            "is_dir": is_dir,
            "size": metadata.as_ref().map_or(0, std::fs::Metadata::len),
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

/// Skill that returns metadata (size, permissions, timestamps) for a path.
pub struct FsInfoSkill {
    guard: Arc<PermissionGuard>,
}

impl FsInfoSkill {
    /// Create a new `fs_info` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for FsInfoSkill {
    fn name(&self) -> &'static str {
        "fs_info"
    }

    fn category(&self) -> &'static str {
        "filesystem"
    }

    fn description(&self) -> &'static str {
        "Get detailed metadata for a file or directory (size, type, permissions, timestamps)."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to inspect" }
            },
            "required": ["path"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let path_str = input
            .args
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| missing_arg("path"))?;

        let path = input.resolve_path(path_str);
        let canonical = self.guard.check_path(&path)?;

        let metadata = tokio::fs::metadata(&canonical)
            .await
            .map_err(|e| fs_io_error(&format!("cannot stat '{}'", canonical.display()), &e))?;
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
            .is_some_and(std::fs::Metadata::is_symlink);

        Ok(SkillOutput::new(serde_json::json!({
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
        })))
    }
}

// ── fs_search ──

/// Skill that searches for files matching a glob pattern within a directory.
pub struct FsSearchSkill {
    guard: Arc<PermissionGuard>,
    max_list_entries: usize,
}

impl FsSearchSkill {
    /// Create a new `fs_search` skill from config and a permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self {
            guard,
            max_list_entries: 1000, // Default max entries
        }
    }
}

#[async_trait]
impl Skill for FsSearchSkill {
    fn name(&self) -> &'static str {
        "fs_search"
    }

    fn category(&self) -> &'static str {
        "filesystem"
    }

    fn description(&self) -> &'static str {
        "Search for files by name (glob), or search file contents for a pattern."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
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
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let path_str = input
            .args
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| missing_arg("path"))?;
        let pattern = input
            .args
            .get("pattern")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| missing_arg("pattern"))?;
        let by = input
            .args
            .get("by")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("glob");
        let max_results = input
            .args
            .get("max_results")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(50) as usize;

        let path = input.resolve_path(path_str);
        let canonical = self.guard.check_path(&path)?;

        let max = max_results.min(self.max_list_entries);

        match by {
            "glob" => {
                let glob_pattern = format!("{}/{}", canonical.display(), pattern);
                let mut results = Vec::new();
                for entry in glob::glob(&glob_pattern).map_err(|e| Error::SkillCategorized {
                    message: format!("invalid glob pattern: {e}"),
                    category: ErrorCategory::Input,
                })? {
                    if results.len() >= max {
                        break;
                    }
                    if let Ok(p) = entry {
                        results.push(serde_json::json!(p.to_string_lossy()));
                    }
                }
                Ok(SkillOutput::new(serde_json::json!({
                    "matches": results,
                    "count": results.len(),
                    "search_type": "glob",
                })))
            }
            "name" => {
                let mut results = Vec::new();
                search_by_name(&canonical, pattern, &mut results, max).await?;
                Ok(SkillOutput::new(serde_json::json!({
                    "matches": results,
                    "count": results.len(),
                    "search_type": "name",
                })))
            }
            "content" => {
                let mut results = Vec::new();
                search_by_content(&canonical, pattern, &mut results, max).await?;
                Ok(SkillOutput::new(serde_json::json!({
                    "matches": results,
                    "count": results.len(),
                    "search_type": "content",
                })))
            }
            _ => Err(Error::SkillCategorized {
                message: format!("unknown search mode: {by}"),
                category: ErrorCategory::Input,
            }),
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
        .map_err(|e| fs_io_error(&format!("cannot read '{}'", dir.display()), &e))?;

    let pat_lower = pattern.to_lowercase();
    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|e| Error::SkillCategorized {
            message: format!("error reading entry: {e}"),
            category: ErrorCategory::Unknown,
        })?
    {
        if results.len() >= max {
            break;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.to_lowercase().contains(&pat_lower) {
            results.push(serde_json::json!(entry.path().to_string_lossy()));
        }
        if entry.metadata().await.is_ok_and(|m| m.is_dir()) && results.len() < max {
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
        .map_err(|e| fs_io_error(&format!("cannot read '{}'", dir.display()), &e))?;

    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|e| Error::SkillCategorized {
            message: format!("error reading entry: {e}"),
            category: ErrorCategory::Unknown,
        })?
    {
        if results.len() >= max {
            break;
        }
        let metadata = entry.metadata().await.ok();
        let is_dir = metadata.as_ref().is_some_and(std::fs::Metadata::is_dir);
        let is_file = metadata.as_ref().is_some_and(std::fs::Metadata::is_file);

        if is_file {
            // Skip large files
            let size = metadata.as_ref().map_or(0, std::fs::Metadata::len);
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

/// Skill that writes text or binary content to a file within the allowed paths.
pub struct FsWriteSkill {
    guard: Arc<PermissionGuard>,
}

impl FsWriteSkill {
    /// Create a new `fs_write` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for FsWriteSkill {
    fn name(&self) -> &'static str {
        "fs_write"
    }

    fn category(&self) -> &'static str {
        "filesystem"
    }

    fn description(&self) -> &'static str {
        "Write content to a file. Supports write (overwrite) and append modes."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to write" },
                "content": { "type": "string", "description": "Content to write" },
                "mode": { "type": "string", "enum": ["write", "append"], "default": "write" },
                "create_dirs": { "type": "boolean", "default": false, "description": "Create parent directories" },
                "encoding": { "type": "string", "enum": ["utf8", "base64"], "default": "utf8" }
            },
            "required": ["path", "content"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Write)?;

        let path_str = input
            .args
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| missing_arg("path"))?;
        let content = input
            .args
            .get("content")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| missing_arg("content"))?;
        let mode = input
            .args
            .get("mode")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("write");
        let create_dirs = input
            .args
            .get("create_dirs")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        self.guard.check_file_size(content.len() as u64)?;

        let path = input.resolve_path(path_str);
        let canonical = self.guard.check_write_path(&path)?;

        if create_dirs && let Some(parent) = canonical.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| fs_io_error("cannot create directories", &e))?;
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
                        fs_io_error(&format!("cannot open '{}'", canonical.display()), &e)
                    })?;
                file.write_all(content.as_bytes())
                    .await
                    .map_err(|e| fs_io_error("write failed", &e))?;
            }
            _ => {
                tokio::fs::write(&canonical, content.as_bytes())
                    .await
                    .map_err(|e| fs_io_error("write failed", &e))?;
            }
        }

        debug!(path = %canonical.display(), bytes = bytes_written, "fs_write complete");

        Ok(SkillOutput::new(serde_json::json!({
            "bytes_written": bytes_written,
            "path": canonical.to_string_lossy(),
        })))
    }
}

// ── fs_edit ──

/// Skill that applies targeted search-and-replace edits to a file.
pub struct FsEditSkill {
    guard: Arc<PermissionGuard>,
}

impl FsEditSkill {
    /// Create a new `fs_edit` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for FsEditSkill {
    fn name(&self) -> &'static str {
        "fs_edit"
    }

    fn category(&self) -> &'static str {
        "filesystem"
    }

    fn description(&self) -> &'static str {
        "Apply targeted search-and-replace edits to a file. Each edit must match exactly once."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to edit" },
                "edits": {
                    "type": "array",
                    "description": "List of search-and-replace edits to apply in order",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_text": { "type": "string", "description": "Exact text to find (must appear exactly once)" },
                            "new_text": { "type": "string", "description": "Text to replace it with" }
                        },
                        "required": ["old_text", "new_text"]
                    },
                    "minItems": 1
                }
            },
            "required": ["path", "edits"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Write)?;

        let path_str = input
            .args
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| missing_arg("path"))?;

        let edits = input
            .args
            .get("edits")
            .and_then(|v| v.as_array())
            .ok_or_else(|| missing_arg("edits"))?;

        let path = input.resolve_path(path_str);
        let canonical = self.guard.check_write_path(&path)?;

        let original = tokio::fs::read_to_string(&canonical)
            .await
            .map_err(|e| fs_io_error(&format!("cannot read '{}'", canonical.display()), &e))?;

        let mut content = original.clone();
        let mut edits_applied = 0usize;

        for (i, edit) in edits.iter().enumerate() {
            let old_text = edit
                .get("old_text")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| Error::SkillCategorized {
                    message: format!("edit[{i}] missing 'old_text'"),
                    category: ErrorCategory::Input,
                })?;
            let new_text = edit
                .get("new_text")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| Error::SkillCategorized {
                    message: format!("edit[{i}] missing 'new_text'"),
                    category: ErrorCategory::Input,
                })?;

            let count = content.matches(old_text).count();
            if count == 0 {
                return Err(Error::SkillCategorized {
                    message: format!(
                        "edit[{i}]: 'old_text' not found in file. File starts with:\n{}",
                        content.lines().take(5).collect::<Vec<_>>().join("\n")
                    ),
                    category: ErrorCategory::Input,
                });
            }
            if count > 1 {
                return Err(Error::SkillCategorized {
                    message: format!(
                        "edit[{i}]: 'old_text' found {count} times — must be unique. Provide more context to make it unambiguous."
                    ),
                    category: ErrorCategory::Input,
                });
            }

            content = content.replacen(old_text, new_text, 1);
            edits_applied += 1;
        }

        tokio::fs::write(&canonical, content.as_bytes())
            .await
            .map_err(|e| fs_io_error("write failed", &e))?;

        debug!(path = %canonical.display(), edits = edits_applied, "fs_edit complete");

        Ok(SkillOutput::new(serde_json::json!({
            "edits_applied": edits_applied,
            "path": canonical.to_string_lossy(),
        })))
    }
}

// ── fs_watch ──

/// Skill that watches a path for filesystem changes (create, modify, delete).
pub struct FsWatchSkill {
    guard: Arc<PermissionGuard>,
}

impl FsWatchSkill {
    /// Create a new `fs_watch` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for FsWatchSkill {
    fn name(&self) -> &'static str {
        "fs_watch"
    }

    fn category(&self) -> &'static str {
        "filesystem"
    }

    fn description(&self) -> &'static str {
        "Watch a path for filesystem changes for a specified duration. Returns batch of events."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to watch" },
                "duration_secs": { "type": "integer", "default": 5, "maximum": 30, "description": "How long to watch" },
                "recursive": { "type": "boolean", "default": true }
            },
            "required": ["path"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Execute)?;

        let path_str = input
            .args
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| missing_arg("path"))?;
        let duration_secs = input
            .args
            .get("duration_secs")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(5)
            .min(30);
        let recursive = input
            .args
            .get("recursive")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);

        use notify::Watcher;

        let path = input.resolve_path(path_str);
        let canonical = self.guard.check_path(&path)?;

        let (tx, mut rx) = tokio::sync::mpsc::channel(256);

        let mut watcher = notify::recommended_watcher(
            move |res: std::result::Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    // Receiver dropped means the watcher is shutting down — safe to ignore.
                    let _ = tx.blocking_send(event);
                }
            },
        )
        .map_err(|e| Error::SkillCategorized {
            message: format!("cannot create watcher: {e}"),
            category: ErrorCategory::Environmental,
        })?;
        let mode = if recursive {
            notify::RecursiveMode::Recursive
        } else {
            notify::RecursiveMode::NonRecursive
        };
        watcher
            .watch(&canonical, mode)
            .map_err(|e| Error::SkillCategorized {
                message: format!("cannot watch '{}': {e}", canonical.display()),
                category: ErrorCategory::Environmental,
            })?;

        let mut events = Vec::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(duration_secs);

        loop {
            tokio::select! {
                () = tokio::time::sleep_until(deadline) => break,
                ev = rx.recv() => {
                    match ev {
                        Some(event) => {
                            events.push(serde_json::json!({
                                "kind": format!("{:?}", event.kind), // event.kind doesn't impl Display
                                "paths": event.paths.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>(),
                            }));
                        }
                        None => break,
                    }
                }
            }
        }

        Ok(SkillOutput::new(serde_json::json!({
            "events": events,
            "count": events.len(),
            "watched_path": canonical.to_string_lossy(),
            "duration_secs": duration_secs,
        })))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use orka_core::config::{OsConfig, primitives::OsPermissionLevel};

    use super::*;

    fn test_guard(level: &str) -> Arc<PermissionGuard> {
        let mut config = OsConfig::default();
        config.permission_level = match level {
            "read-only" => OsPermissionLevel::ReadOnly,
            "write" => OsPermissionLevel::Write,
            other => panic!("unsupported test permission level: {other}"),
        };
        config.allowed_paths = vec!["/tmp".into()];
        Arc::new(PermissionGuard::new(&config))
    }

    #[test]
    fn fs_read_schema_valid() {
        let skill = FsReadSkill::new(test_guard("read-only"));
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "path");
    }

    #[tokio::test]
    async fn fs_read_happy_path() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "hello world").unwrap();

        let mut config = OsConfig::default();
        config.allowed_paths = vec![dir.path().to_string_lossy().to_string()];
        let guard = Arc::new(PermissionGuard::new(&config));
        let skill = FsReadSkill::new(guard);

        let mut args = HashMap::new();
        args.insert("path".into(), serde_json::json!(file.to_string_lossy()));
        let output = skill.execute(SkillInput::new(args)).await.unwrap();
        assert_eq!(output.data["content"], "hello world");
        assert_eq!(output.data["encoding"], "utf8");
    }

    #[tokio::test]
    async fn fs_read_missing_path_errors() {
        let skill = FsReadSkill::new(test_guard("read-only"));
        let input = SkillInput::new(HashMap::new());
        assert!(skill.execute(input).await.is_err());
    }

    #[tokio::test]
    async fn fs_list_happy_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "").unwrap();
        std::fs::write(dir.path().join("b.txt"), "").unwrap();

        let mut config = OsConfig::default();
        config.allowed_paths = vec![dir.path().to_string_lossy().to_string()];
        let guard = Arc::new(PermissionGuard::new(&config));
        let skill = FsListSkill::new(guard);

        let mut args = HashMap::new();
        args.insert(
            "path".into(),
            serde_json::json!(dir.path().to_string_lossy()),
        );
        let output = skill.execute(SkillInput::new(args)).await.unwrap();
        assert_eq!(output.data["total"], 2);
    }

    #[tokio::test]
    async fn fs_write_requires_write_permission() {
        let guard = test_guard("read-only");
        let skill = FsWriteSkill::new(guard);

        let mut args = HashMap::new();
        args.insert("path".into(), serde_json::json!("/tmp/test_write.txt"));
        args.insert("content".into(), serde_json::json!("data"));
        let result = skill.execute(SkillInput::new(args)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fs_write_happy_path() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("out.txt");

        let mut config = OsConfig::default();
        config.permission_level = OsPermissionLevel::Write;
        config.allowed_paths = vec![dir.path().to_string_lossy().to_string()];
        let guard = Arc::new(PermissionGuard::new(&config));
        let skill = FsWriteSkill::new(guard);

        let mut args = HashMap::new();
        args.insert("path".into(), serde_json::json!(file.to_string_lossy()));
        args.insert("content".into(), serde_json::json!("hello"));
        let output = skill.execute(SkillInput::new(args)).await.unwrap();
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
        let skill = FsSearchSkill::new(test_guard("read-only"));
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "path");
        assert_eq!(schema.parameters["required"][1], "pattern");
    }
}
