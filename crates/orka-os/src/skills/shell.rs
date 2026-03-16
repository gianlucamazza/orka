use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use orka_core::config::OsConfig;
use orka_core::traits::Skill;
use orka_core::{Error, Result, SkillInput, SkillOutput, SkillSchema};
use tracing::debug;

use crate::config::PermissionLevel;
use crate::guard::PermissionGuard;

pub struct ShellExecSkill {
    guard: Arc<PermissionGuard>,
    timeout_secs: u64,
    max_output_bytes: usize,
}

impl ShellExecSkill {
    pub fn new(guard: Arc<PermissionGuard>, config: &OsConfig) -> Self {
        Self {
            guard,
            timeout_secs: config.shell_timeout_secs,
            max_output_bytes: config.max_output_bytes,
        }
    }
}

#[async_trait]
impl Skill for ShellExecSkill {
    fn name(&self) -> &str {
        "shell_exec"
    }

    fn description(&self) -> &str {
        "Execute a command directly (no shell interpretation). Returns exit code, stdout, and stderr."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Command to execute (no shell interpretation)" },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Command arguments"
                    },
                    "cwd": { "type": "string", "description": "Working directory" },
                    "env": {
                        "type": "object",
                        "additionalProperties": { "type": "string" },
                        "description": "Additional environment variables"
                    },
                    "timeout_secs": { "type": "integer", "description": "Execution timeout in seconds" },
                    "stdin": { "type": "string", "description": "Standard input to provide" }
                },
                "required": ["command"]
            }),
        }
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        self.guard.check_permission(PermissionLevel::Execute)?;

        let command = input
            .args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Skill("missing 'command' argument".into()))?;
        let args: Vec<&str> = input
            .args
            .get("args")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        let cwd = input.args.get("cwd").and_then(|v| v.as_str());
        let env_vars = input
            .args
            .get("env")
            .and_then(|v| v.as_object());
        let timeout = input
            .args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(self.timeout_secs);
        let stdin_data = input.args.get("stdin").and_then(|v| v.as_str());

        // Validate command
        self.guard.check_command(command, &args)?;

        // Validate cwd if provided
        if let Some(dir) = cwd {
            self.guard.check_path(Path::new(dir))?;
        }

        debug!(command, ?args, "shell_exec");

        let mut cmd = tokio::process::Command::new(command);
        cmd.args(&args);
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
            Error::Skill(format!("failed to spawn '{}': {}", command, e))
        })?;

        // Write stdin if provided
        if let Some(data) = stdin_data {
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let _ = stdin.write_all(data.as_bytes()).await;
                drop(stdin);
            }
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

                Ok(SkillOutput {
                    data: serde_json::json!({
                        "exit_code": output.status.code(),
                        "stdout": stdout,
                        "stderr": stderr,
                        "duration_ms": duration_ms,
                    }),
                })
            }
            Ok(Err(e)) => Err(Error::Skill(format!("command execution failed: {}", e))),
            Err(_) => {
                // Process timed out — try to kill by PID
                if let Some(pid) = child_id {
                    let _ = nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(pid as i32),
                        nix::sys::signal::Signal::SIGKILL,
                    );
                }
                Err(Error::Skill(format!(
                    "command timed out after {} seconds",
                    timeout
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_skill() -> ShellExecSkill {
        use orka_core::config::OsConfig;
        let config = OsConfig {
            permission_level: "execute".into(),
            ..OsConfig::default()
        };
        ShellExecSkill::new(Arc::new(PermissionGuard::new(&config)), &config)
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
        let output = skill
            .execute(SkillInput { args, context: None })
            .await
            .unwrap();
        assert_eq!(output.data["exit_code"], 0);
        assert!(output.data["stdout"]
            .as_str()
            .unwrap()
            .contains("hello world"));
    }

    #[tokio::test]
    async fn exec_requires_execute_permission() {
        use orka_core::config::OsConfig;
        let config = OsConfig {
            permission_level: "read-only".into(),
            ..OsConfig::default()
        };
        let skill = ShellExecSkill::new(Arc::new(PermissionGuard::new(&config)), &config);
        let mut args = HashMap::new();
        args.insert("command".into(), serde_json::json!("echo"));
        assert!(skill
            .execute(SkillInput { args, context: None })
            .await
            .is_err());
    }

    #[tokio::test]
    async fn exec_blocked_command() {
        use orka_core::config::OsConfig;
        let config = OsConfig {
            permission_level: "execute".into(),
            blocked_commands: vec!["rm".into()],
            ..OsConfig::default()
        };
        let skill = ShellExecSkill::new(Arc::new(PermissionGuard::new(&config)), &config);
        let mut args = HashMap::new();
        args.insert("command".into(), serde_json::json!("rm"));
        args.insert("args".into(), serde_json::json!(["-rf", "/"]));
        assert!(skill
            .execute(SkillInput { args, context: None })
            .await
            .is_err());
    }

    #[tokio::test]
    async fn exec_missing_command_errors() {
        let skill = make_skill();
        let input = SkillInput {
            args: HashMap::new(),
            context: None,
        };
        assert!(skill.execute(input).await.is_err());
    }
}
