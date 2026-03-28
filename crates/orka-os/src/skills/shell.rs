use std::{path::Path, sync::Arc};

use async_trait::async_trait;
use orka_core::{
    Error, ErrorCategory, Result, SkillInput, SkillOutput, SkillSchema, traits::Skill,
};
use tracing::debug;

use crate::{config::PermissionLevel, events::emit_executed, guard::PermissionGuard};

/// Skill that executes shell commands with permission and approval enforcement.
pub struct ShellExecSkill {
    guard: Arc<PermissionGuard>,
    timeout_secs: u64,
    max_output_bytes: usize,
}

impl ShellExecSkill {
    /// Create a new `shell_exec` skill from a permission guard.
    pub fn new(guard: Arc<PermissionGuard>) -> Self {
        Self {
            guard,
            timeout_secs: 30,             // Default timeout
            max_output_bytes: 100 * 1024, // Default 100KB
        }
    }
}

#[allow(clippy::too_many_lines)]
#[async_trait]
impl Skill for ShellExecSkill {
    fn name(&self) -> &'static str {
        "shell_exec"
    }

    fn category(&self) -> &'static str {
        "shell"
    }

    fn description(&self) -> &'static str {
        "Execute a command directly (no shell interpretation). Returns exit code, stdout, and stderr."
    }

    fn schema(&self) -> SkillSchema {
        let mut props = serde_json::json!({
            "command": { "type": "string", "description": "Command to execute (no shell interpretation)" },
            "args": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Command arguments"
            },
            "cwd": { "type": "string", "description": "Working directory (defaults to the user's current working directory when omitted)" },
            "env": {
                "type": "object",
                "additionalProperties": { "type": "string" },
                "description": "Additional environment variables"
            },
            "timeout_secs": { "type": "integer", "description": "Execution timeout in seconds" },
            "stdin": { "type": "string", "description": "Standard input to provide" }
        });

        // Only expose sudo parameter to Admin-level users when sudo is enabled
        if self.guard.sudo_enabled() && self.guard.level() >= PermissionLevel::Admin {
            #[allow(clippy::unwrap_used)] // props is a json!({}) object literal, always an object
            props.as_object_mut().unwrap().insert(
                "sudo".into(),
                serde_json::json!({
                    "type": "boolean",
                    "description": "Execute with elevated privileges via sudo (requires admin level and allowed command)"
                }),
            );
        }

        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": props,
            "required": ["command"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Execute)?;

        let raw_command = input
            .args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'command' argument".into(),
                category: ErrorCategory::Input,
            })?;
        let explicit_args: Vec<&str> = input
            .args
            .get("args")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        // When the LLM packs command+args into a single string (e.g. "df -h /"),
        // split it with POSIX shell quoting rules so Command::new gets the right
        // binary.
        let (command_owned, args_owned) = if explicit_args.is_empty() && raw_command.contains(' ') {
            let mut parts =
                shell_words::split(raw_command).map_err(|e| Error::SkillCategorized {
                    message: format!("failed to parse command string: {e}"),
                    category: ErrorCategory::Input,
                })?;
            if parts.is_empty() {
                return Err(Error::SkillCategorized {
                    message: "empty command after parsing".into(),
                    category: ErrorCategory::Input,
                });
            }
            let cmd = parts.remove(0);
            debug!(raw_command, cmd, "split compound command string");
            (Some(cmd), parts)
        } else {
            (None, Vec::new())
        };
        let command: &str = command_owned.as_deref().unwrap_or(raw_command);
        let args: Vec<&str> = if args_owned.is_empty() {
            explicit_args
        } else {
            args_owned.iter().map(String::as_str).collect()
        };
        let cwd = input
            .args
            .get("cwd")
            .and_then(|v| v.as_str())
            // Prefer active worktree context over the raw user_cwd so commands
            // run inside the agent's isolated worktree automatically.
            .or_else(|| input.context.as_ref().and_then(|c| c.worktree_cwd.as_deref()))
            .or_else(|| input.context.as_ref().and_then(|c| c.user_cwd.as_deref()));
        let env_vars = input.args.get("env").and_then(|v| v.as_object());
        let timeout = input
            .args
            .get("timeout_secs")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(self.timeout_secs);
        let stdin_data = input.args.get("stdin").and_then(|v| v.as_str());
        let use_sudo = input
            .args
            .get("sudo")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        if use_sudo {
            // Sudo flow: Admin check + allowlist
            self.guard.check_sudo_command(command, &args)?;

            // Sudo execution proceeds without interactive approval
            // (approval should be handled externally via sudoers configuration)
        } else {
            // Normal flow: just validate command against allow lists
            self.guard.check_command(command, &args)?;
        }

        // Validate cwd if provided
        if let Some(dir) = cwd {
            self.guard.check_path(Path::new(dir))?;
        }

        debug!(command, ?args, use_sudo, "shell_exec");

        let mut cmd = if use_sudo {
            let mut c = tokio::process::Command::new(self.guard.sudo_path());
            c.arg("-n").arg(command);
            c.args(&args);
            c
        } else {
            let mut c = tokio::process::Command::new(command);
            c.args(&args);
            c
        };
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        if let Some(vars) = env_vars {
            for (k, v) in vars {
                if let Some(val) = v.as_str() {
                    cmd.env(k, val);
                }
            }
        }

        if stdin_data.is_some() {
            cmd.stdin(std::process::Stdio::piped());
        }
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let start = std::time::Instant::now();

        let mut child = cmd.spawn().map_err(|e| {
            let category = match e.kind() {
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
                    ErrorCategory::Environmental
                }
                _ => ErrorCategory::Unknown,
            };
            Error::SkillCategorized {
                message: format!("failed to spawn '{command}': {e}"),
                category,
            }
        })?;

        // Write stdin if provided
        if let Some(data) = stdin_data
            && let Some(mut stdin) = child.stdin.take()
        {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(data.as_bytes()).await;
            drop(stdin);
        }

        let child_id = child.id();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout),
            child.wait_with_output(),
        )
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(output)) => {
                let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();

                if stdout.len() > self.max_output_bytes {
                    stdout.truncate(self.max_output_bytes);
                    stdout.push_str("\n... [truncated]");
                }
                if stderr.len() > self.max_output_bytes {
                    stderr.truncate(self.max_output_bytes);
                    stderr.push_str("\n... [truncated]");
                }

                if use_sudo {
                    emit_executed(
                        &input,
                        command,
                        &args,
                        output.status.code(),
                        output.status.success(),
                        duration_ms,
                    )
                    .await;
                }

                Ok(SkillOutput::new(serde_json::json!({
                    "exit_code": output.status.code(),
                    "stdout": stdout,
                    "stderr": stderr,
                    "duration_ms": duration_ms,
                })))
            }
            Ok(Err(e)) => Err(Error::SkillCategorized {
                message: format!("command execution failed: {e}"),
                category: ErrorCategory::Unknown,
            }),
            Err(_) => {
                // Process timed out — try to kill by PID
                if let Some(pid) = child_id {
                    let _ = nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(pid as i32),
                        nix::sys::signal::Signal::SIGKILL,
                    );
                }
                Err(Error::SkillCategorized {
                    message: format!("command timed out after {timeout} seconds"),
                    category: ErrorCategory::Timeout,
                })
            }
        }
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
    use std::collections::HashMap;

    use super::*;
    use crate::config::{OsConfig, PermissionLevel, SudoConfig};

    fn make_skill() -> ShellExecSkill {
        let mut config = OsConfig::default();
        config.permission_level = PermissionLevel::Execute;
        ShellExecSkill::new(Arc::new(PermissionGuard::new(&config)))
    }

    #[test]
    fn schema_is_valid() {
        let skill = make_skill();
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "command");
    }

    #[tokio::test]
    async fn exec_echo() {
        let skill = make_skill();
        let mut args = HashMap::new();
        args.insert("command".into(), serde_json::json!("echo"));
        args.insert("args".into(), serde_json::json!(["hello", "world"]));
        let output = skill.execute(SkillInput::new(args)).await.unwrap();
        assert_eq!(output.data["exit_code"], 0);
        assert!(
            output.data["stdout"]
                .as_str()
                .unwrap()
                .contains("hello world")
        );
    }

    #[tokio::test]
    async fn exec_requires_execute_permission() {
        let mut config = OsConfig::default();
        config.permission_level = PermissionLevel::ReadOnly;
        let skill = ShellExecSkill::new(Arc::new(PermissionGuard::new(&config)));
        let mut args = HashMap::new();
        args.insert("command".into(), serde_json::json!("echo"));
        assert!(skill.execute(SkillInput::new(args)).await.is_err());
    }

    #[tokio::test]
    async fn exec_blocked_command() {
        let mut config = OsConfig::default();
        config.permission_level = PermissionLevel::Execute;
        config.allowed_shell_commands = vec!["echo".into()];
        let skill = ShellExecSkill::new(Arc::new(PermissionGuard::new(&config)));
        let mut args = HashMap::new();
        args.insert("command".into(), serde_json::json!("rm"));
        args.insert("args".into(), serde_json::json!(["-rf", "/"]));
        assert!(skill.execute(SkillInput::new(args)).await.is_err());
    }

    #[tokio::test]
    async fn exec_missing_command_errors() {
        let skill = make_skill();
        let input = SkillInput::new(HashMap::new());
        assert!(skill.execute(input).await.is_err());
    }

    #[tokio::test]
    async fn exec_sudo_requires_admin() {
        let mut config = OsConfig::default();
        config.permission_level = PermissionLevel::Execute;
        let mut sudo = SudoConfig::default();
        sudo.allowed = true;
        sudo.allowed_commands = vec!["echo".into()];
        sudo.password_required = false;
        config.sudo = sudo;
        let skill = ShellExecSkill::new(Arc::new(PermissionGuard::new(&config)));
        let mut args = HashMap::new();
        args.insert("command".into(), serde_json::json!("echo"));
        args.insert("args".into(), serde_json::json!(["hi"]));
        args.insert("sudo".into(), serde_json::json!(true));
        assert!(skill.execute(SkillInput::new(args)).await.is_err());
    }

    #[test]
    fn schema_includes_sudo_for_admin() {
        let mut config = OsConfig::default();
        config.permission_level = PermissionLevel::Admin;
        let mut sudo = SudoConfig::default();
        sudo.allowed = true;
        config.sudo = sudo;
        let skill = ShellExecSkill::new(Arc::new(PermissionGuard::new(&config)));
        let schema = skill.schema();
        assert!(schema.parameters["properties"]["sudo"].is_object());
    }

    #[tokio::test]
    async fn exec_split_compound_command() {
        let skill = make_skill();
        let mut args = HashMap::new();
        args.insert("command".into(), serde_json::json!("echo hello world"));
        let output = skill.execute(SkillInput::new(args)).await.unwrap();
        assert_eq!(output.data["exit_code"], 0);
        assert!(
            output.data["stdout"]
                .as_str()
                .unwrap()
                .contains("hello world")
        );
    }

    #[tokio::test]
    async fn exec_split_preserves_quotes() {
        let skill = make_skill();
        let mut args = HashMap::new();
        args.insert("command".into(), serde_json::json!("echo 'hello world'"));
        let output = skill.execute(SkillInput::new(args)).await.unwrap();
        assert_eq!(output.data["exit_code"], 0);
        assert!(
            output.data["stdout"]
                .as_str()
                .unwrap()
                .contains("hello world")
        );
    }

    #[tokio::test]
    async fn exec_no_split_when_args_present() {
        let skill = make_skill();
        let mut args = HashMap::new();
        args.insert("command".into(), serde_json::json!("echo"));
        args.insert("args".into(), serde_json::json!(["hello"]));
        let output = skill.execute(SkillInput::new(args)).await.unwrap();
        assert_eq!(output.data["exit_code"], 0);
        assert!(output.data["stdout"].as_str().unwrap().contains("hello"));
    }

    #[tokio::test]
    async fn exec_split_blocked_command() {
        let mut config = OsConfig::default();
        config.permission_level = PermissionLevel::Execute;
        config.allowed_shell_commands = vec!["echo".into()];
        let skill = ShellExecSkill::new(Arc::new(PermissionGuard::new(&config)));
        let mut args = HashMap::new();
        args.insert("command".into(), serde_json::json!("rm -rf /"));
        assert!(skill.execute(SkillInput::new(args)).await.is_err());
    }

    #[test]
    fn schema_hides_sudo_for_non_admin() {
        let mut config = OsConfig::default();
        config.permission_level = PermissionLevel::Execute;
        let mut sudo = SudoConfig::default();
        sudo.allowed = true;
        config.sudo = sudo;
        let skill = ShellExecSkill::new(Arc::new(PermissionGuard::new(&config)));
        let schema = skill.schema();
        assert!(schema.parameters["properties"]["sudo"].is_null());
    }
}
