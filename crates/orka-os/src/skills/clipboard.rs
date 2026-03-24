use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{
    Error, ErrorCategory, Result, SkillInput, SkillOutput, SkillSchema, traits::Skill,
};

use crate::{config::PermissionLevel, guard::PermissionGuard};

fn is_wayland() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
}

// ── clipboard_read ──

/// Skill that reads the current clipboard contents.
pub struct ClipboardReadSkill {
    guard: Arc<PermissionGuard>,
}

impl ClipboardReadSkill {
    /// Create a new `clipboard_read` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for ClipboardReadSkill {
    fn name(&self) -> &str {
        "clipboard_read"
    }

    fn category(&self) -> &str {
        "desktop"
    }

    fn description(&self) -> &str {
        "Read the current system clipboard contents."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        }))
    }

    async fn execute(&self, _input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Interact)?;

        let (cmd, args) = if is_wayland() {
            ("wl-paste", vec!["--no-newline"])
        } else {
            ("xclip", vec!["-selection", "clipboard", "-o"])
        };

        let output = tokio::process::Command::new(cmd)
            .args(&args)
            .output()
            .await
            .map_err(|e| Error::SkillCategorized {
                message: format!("clipboard read failed ({}): {}", cmd, e),
                category: ErrorCategory::Environmental,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::SkillCategorized {
                message: format!(
                    "clipboard_read failed (exit {}): {}",
                    output.status.code().unwrap_or(-1),
                    stderr.trim()
                ),
                category: ErrorCategory::Environmental,
            });
        }

        let content = String::from_utf8_lossy(&output.stdout).to_string();

        Ok(SkillOutput::new(serde_json::json!({
            "content": content,
            "backend": if is_wayland() { "wayland" } else { "x11" },
        })))
    }
}

// ── clipboard_write ──

/// Skill that writes text to the system clipboard.
pub struct ClipboardWriteSkill {
    guard: Arc<PermissionGuard>,
}

impl ClipboardWriteSkill {
    /// Create a new `clipboard_write` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for ClipboardWriteSkill {
    fn name(&self) -> &str {
        "clipboard_write"
    }

    fn category(&self) -> &str {
        "desktop"
    }

    fn description(&self) -> &str {
        "Write content to the system clipboard."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "Content to copy to clipboard" }
            },
            "required": ["content"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Interact)?;

        let content = input
            .args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'content' argument".into(),
                category: ErrorCategory::Input,
            })?;

        let (cmd, args) = if is_wayland() {
            ("wl-copy", vec![])
        } else {
            ("xclip", vec!["-selection", "clipboard"])
        };

        let mut child = tokio::process::Command::new(cmd)
            .args(&args)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| Error::SkillCategorized {
                message: format!("clipboard write failed ({}): {}", cmd, e),
                category: ErrorCategory::Environmental,
            })?;

        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(content.as_bytes())
                .await
                .map_err(|e| Error::SkillCategorized {
                    message: format!("failed to write to clipboard: {}", e),
                    category: ErrorCategory::Environmental,
                })?;
        }

        let status = child.wait().await.map_err(|e| Error::SkillCategorized {
            message: format!("clipboard command failed: {}", e),
            category: ErrorCategory::Environmental,
        })?;

        if !status.success() {
            return Err(Error::SkillCategorized {
                message: format!(
                    "clipboard_write failed (exit {})",
                    status.code().unwrap_or(-1)
                ),
                category: ErrorCategory::Environmental,
            });
        }

        Ok(SkillOutput::new(serde_json::json!({
            "bytes_written": content.len(),
            "backend": if is_wayland() { "wayland" } else { "x11" },
        })))
    }
}

#[cfg(test)]
mod tests {
    use orka_core::config::{OsConfig, primitives::OsPermissionLevel};

    use super::*;

    fn make_guard() -> Arc<PermissionGuard> {
        let mut config = OsConfig::default();
        config.permission_level = OsPermissionLevel::Interact;
        Arc::new(PermissionGuard::new(&config))
    }

    #[test]
    fn clipboard_read_schema_valid() {
        let skill = ClipboardReadSkill::new(make_guard());
        let _schema = skill.schema();
    }

    #[test]
    fn clipboard_write_schema_valid() {
        let skill = ClipboardWriteSkill::new(make_guard());
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "content");
    }

    #[tokio::test]
    async fn clipboard_read_requires_interact_permission() {
        let mut config = OsConfig::default();
        config.permission_level = OsPermissionLevel::ReadOnly;
        let guard = Arc::new(PermissionGuard::new(&config));
        let skill = ClipboardReadSkill::new(guard);
        let input = SkillInput::new(std::collections::HashMap::new());
        assert!(skill.execute(input).await.is_err());
    }
}
