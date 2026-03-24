use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{
    Error, ErrorCategory, Result, SkillInput, SkillOutput, SkillSchema, traits::Skill,
};

use crate::{config::PermissionLevel, guard::PermissionGuard};

fn is_wayland() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
}

// ── desktop_open ──

/// Skill that opens a file or URL with the default desktop application.
pub struct DesktopOpenSkill {
    guard: Arc<PermissionGuard>,
}

impl DesktopOpenSkill {
    /// Create a new `desktop_open` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for DesktopOpenSkill {
    fn name(&self) -> &str {
        "desktop_open"
    }

    fn category(&self) -> &str {
        "desktop"
    }

    fn description(&self) -> &str {
        "Open a file or URL with the default application using xdg-open."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "target": { "type": "string", "description": "URL or file path to open" }
            },
            "required": ["target"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Execute)?;

        let target = input
            .args
            .get("target")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'target' argument".into(),
                category: ErrorCategory::Input,
            })?;

        // Validate: must be a URL or file path
        let is_url = target.starts_with("http://")
            || target.starts_with("https://")
            || target.starts_with("file://")
            || target.starts_with("mailto:");

        if !is_url {
            // Validate as file path, resolving relative paths against user_cwd
            let path = input.resolve_path(target);
            self.guard.check_path(&path)?;
        }

        let output = tokio::process::Command::new("xdg-open")
            .arg(target)
            .output()
            .await
            .map_err(|e| Error::SkillCategorized {
                message: format!("xdg-open failed: {}", e),
                category: ErrorCategory::Environmental,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::SkillCategorized {
                message: format!(
                    "xdg-open failed (exit {}): {}",
                    output.status.code().unwrap_or(-1),
                    stderr.trim()
                ),
                category: ErrorCategory::Environmental,
            });
        }

        Ok(SkillOutput::new(serde_json::json!({
            "success": true,
            "target": target,
        })))
    }
}

// ── desktop_screenshot ──

/// Skill that captures a screenshot of the current desktop.
pub struct DesktopScreenshotSkill {
    guard: Arc<PermissionGuard>,
}

impl DesktopScreenshotSkill {
    /// Create a new `desktop_screenshot` skill with the given permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl Skill for DesktopScreenshotSkill {
    fn name(&self) -> &str {
        "desktop_screenshot"
    }

    fn category(&self) -> &str {
        "desktop"
    }

    fn description(&self) -> &str {
        "Take a screenshot of the desktop. Uses grim (Wayland) or scrot (X11)."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "output_path": { "type": "string", "description": "Path to save screenshot (default: /tmp/screenshot.png)" },
                "region": {
                    "type": "string",
                    "enum": ["full", "window", "selection"],
                    "default": "full"
                }
            },
            "required": []
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Execute)?;

        let output_path = input
            .args
            .get("output_path")
            .and_then(|v| v.as_str())
            .unwrap_or("/tmp/screenshot.png");
        let region = input
            .args
            .get("region")
            .and_then(|v| v.as_str())
            .unwrap_or("full");

        // Validate output path
        self.guard
            .check_write_path(std::path::Path::new(output_path))?;

        let output = if is_wayland() {
            let mut cmd = tokio::process::Command::new("grim");
            if region == "selection" {
                // Run slurp separately to get the selection geometry, then pass it to grim.
                let slurp = tokio::process::Command::new("slurp")
                    .output()
                    .await
                    .map_err(|e| Error::SkillCategorized {
                        message: format!("slurp failed: {}", e),
                        category: ErrorCategory::Environmental,
                    })?;
                if !slurp.status.success() {
                    return Err(Error::SkillCategorized {
                        message: "slurp selection cancelled".into(),
                        category: ErrorCategory::Input,
                    });
                }
                let geometry = String::from_utf8_lossy(&slurp.stdout).trim().to_string();
                cmd.arg("-g").arg(geometry);
            }
            cmd.arg(output_path);
            cmd.output().await
        } else {
            let mut cmd = tokio::process::Command::new("scrot");
            match region {
                "window" => {
                    cmd.arg("--focused");
                }
                "selection" => {
                    cmd.arg("--select");
                }
                _ => {}
            }
            cmd.arg(output_path);
            cmd.output().await
        };

        let output = output.map_err(|e| Error::SkillCategorized {
            message: format!("screenshot failed: {}", e),
            category: ErrorCategory::Environmental,
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::SkillCategorized {
                message: format!(
                    "screenshot command failed (exit {}): {}",
                    output.status.code().unwrap_or(-1),
                    stderr.trim()
                ),
                category: ErrorCategory::Environmental,
            });
        }

        Ok(SkillOutput::new(serde_json::json!({
            "success": true,
            "path": output_path,
            "backend": if is_wayland() { "grim" } else { "scrot" },
            "region": region,
        })))
    }
}

#[cfg(test)]
mod tests {
    use orka_core::config::{OsConfig, primitives::OsPermissionLevel};

    use super::*;

    fn make_guard() -> Arc<PermissionGuard> {
        let mut config = OsConfig::default();
        config.permission_level = OsPermissionLevel::Execute;
        config.allowed_paths = vec!["/tmp".into()];
        Arc::new(PermissionGuard::new(&config))
    }

    #[test]
    fn desktop_open_schema_valid() {
        let skill = DesktopOpenSkill::new(make_guard());
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "target");
    }

    #[test]
    fn desktop_screenshot_schema_valid() {
        let skill = DesktopScreenshotSkill::new(make_guard());
        let _schema = skill.schema();
    }

    #[tokio::test]
    async fn desktop_open_requires_execute() {
        let mut config = OsConfig::default();
        config.permission_level = OsPermissionLevel::ReadOnly;
        let guard = Arc::new(PermissionGuard::new(&config));
        let skill = DesktopOpenSkill::new(guard);
        let mut args = std::collections::HashMap::new();
        args.insert("target".into(), serde_json::json!("https://example.com"));
        assert!(skill.execute(SkillInput::new(args)).await.is_err());
    }
}
