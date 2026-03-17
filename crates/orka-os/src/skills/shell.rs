use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use orka_core::config::OsConfig;
use orka_core::traits::Skill;
use orka_core::{
    DomainEvent, DomainEventKind, Error, Result, SkillInput, SkillOutput, SkillSchema,
};
use tracing::debug;
use uuid::Uuid;

use crate::approval::{ApprovalChannel, ApprovalDecision, ApprovalRequest};
use crate::config::PermissionLevel;
use crate::guard::PermissionGuard;

pub struct ShellExecSkill {
    guard: Arc<PermissionGuard>,
    timeout_secs: u64,
    max_output_bytes: usize,
    require_confirmation: bool,
    confirmation_timeout_secs: u64,
    approval: Arc<dyn ApprovalChannel>,
}

impl ShellExecSkill {
    pub fn new(
        guard: Arc<PermissionGuard>,
        config: &OsConfig,
        approval: Arc<dyn ApprovalChannel>,
    ) -> Self {
        Self {
            guard,
            timeout_secs: config.shell_timeout_secs,
            max_output_bytes: config.max_output_bytes,
            require_confirmation: config.sudo.require_confirmation,
            confirmation_timeout_secs: config.sudo.confirmation_timeout_secs,
            approval,
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
        let mut props = serde_json::json!({
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
        });

        // Only expose sudo parameter to Admin-level users when sudo is enabled
        if self.guard.sudo_enabled() && self.guard.level() >= PermissionLevel::Admin {
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
        let env_vars = input.args.get("env").and_then(|v| v.as_object());
        let timeout = input
            .args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(self.timeout_secs);
        let stdin_data = input.args.get("stdin").and_then(|v| v.as_str());
        let use_sudo = input
            .args
            .get("sudo")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if use_sudo {
            // Sudo flow: Admin check + allowlist + approval
            self.guard.check_sudo_command(command, &args)?;

            if self.require_confirmation {
                let now = Utc::now();
                let req = ApprovalRequest {
                    id: Uuid::now_v7(),
                    command: command.to_string(),
                    args: args.iter().map(|s| s.to_string()).collect(),
                    reason: format!("sudo execution of: {} {}", command, args.join(" ")),
                    session_id: orka_core::types::SessionId::new(),
                    message_id: orka_core::types::MessageId::new(),
                    requested_at: now,
                    expires_at: now
                        + chrono::Duration::seconds(self.confirmation_timeout_secs as i64),
                };
                match self.approval.request_approval(req).await? {
                    ApprovalDecision::Approved => {}
                    ApprovalDecision::Denied { reason } => {
                        emit_denied(
                            &input,
                            command,
                            &args,
                            &format!("sudo execution denied: {reason}"),
                        )
                        .await;
                        return Err(Error::Skill(format!("sudo execution denied: {}", reason)));
                    }
                    ApprovalDecision::Expired => {
                        emit_denied(&input, command, &args, "sudo approval request expired").await;
                        return Err(Error::Skill("sudo approval request expired".into()));
                    }
                }
            }
        } else {
            // Normal flow: just validate command against block/allow lists
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

        let mut child = cmd
            .spawn()
            .map_err(|e| Error::Skill(format!("failed to spawn '{}': {}", command, e)))?;

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

async fn emit_executed(
    input: &SkillInput,
    command: &str,
    args: &[&str],
    exit_code: Option<i32>,
    success: bool,
    duration_ms: u64,
) {
    if let Some(sink) = input.context.as_ref().and_then(|c| c.event_sink.as_ref()) {
        sink.emit(DomainEvent::new(
            DomainEventKind::PrivilegedCommandExecuted {
                message_id: orka_core::types::MessageId::new(),
                session_id: orka_core::types::SessionId::new(),
                command: command.to_string(),
                args: args.iter().map(|s| s.to_string()).collect(),
                approval_id: None,
                approved_by: None,
                exit_code,
                success,
                duration_ms,
            },
        ))
        .await;
    }
}

async fn emit_denied(input: &SkillInput, command: &str, args: &[&str], reason: &str) {
    if let Some(sink) = input.context.as_ref().and_then(|c| c.event_sink.as_ref()) {
        sink.emit(DomainEvent::new(DomainEventKind::PrivilegedCommandDenied {
            message_id: orka_core::types::MessageId::new(),
            session_id: orka_core::types::SessionId::new(),
            command: command.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            reason: reason.to_string(),
        }))
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_skill() -> ShellExecSkill {
        use crate::approval::AutoApproveChannel;
        use orka_core::config::OsConfig;
        let config = OsConfig {
            permission_level: "execute".into(),
            ..OsConfig::default()
        };
        ShellExecSkill::new(
            Arc::new(PermissionGuard::new(&config)),
            &config,
            Arc::new(AutoApproveChannel),
        )
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
        use crate::approval::AutoApproveChannel;
        use orka_core::config::OsConfig;
        let config = OsConfig {
            permission_level: "read-only".into(),
            ..OsConfig::default()
        };
        let skill = ShellExecSkill::new(
            Arc::new(PermissionGuard::new(&config)),
            &config,
            Arc::new(AutoApproveChannel),
        );
        let mut args = HashMap::new();
        args.insert("command".into(), serde_json::json!("echo"));
        assert!(skill.execute(SkillInput::new(args)).await.is_err());
    }

    #[tokio::test]
    async fn exec_blocked_command() {
        use crate::approval::AutoApproveChannel;
        use orka_core::config::OsConfig;
        let config = OsConfig {
            permission_level: "execute".into(),
            blocked_commands: vec!["rm".into()],
            ..OsConfig::default()
        };
        let skill = ShellExecSkill::new(
            Arc::new(PermissionGuard::new(&config)),
            &config,
            Arc::new(AutoApproveChannel),
        );
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
        use crate::approval::AutoApproveChannel;
        use orka_core::config::{OsConfig, SudoConfig};
        let config = OsConfig {
            permission_level: "execute".into(),
            sudo: SudoConfig {
                enabled: true,
                allowed_commands: vec!["echo".into()],
                require_confirmation: false,
                ..SudoConfig::default()
            },
            ..OsConfig::default()
        };
        let skill = ShellExecSkill::new(
            Arc::new(PermissionGuard::new(&config)),
            &config,
            Arc::new(AutoApproveChannel),
        );
        let mut args = HashMap::new();
        args.insert("command".into(), serde_json::json!("echo"));
        args.insert("args".into(), serde_json::json!(["hi"]));
        args.insert("sudo".into(), serde_json::json!(true));
        assert!(skill.execute(SkillInput::new(args)).await.is_err());
    }

    #[tokio::test]
    async fn exec_sudo_denied_by_approval() {
        use crate::approval::{ApprovalChannel, ApprovalDecision, ApprovalRequest};
        use orka_core::config::{OsConfig, SudoConfig};

        struct DenyChannel;
        #[async_trait]
        impl ApprovalChannel for DenyChannel {
            async fn request_approval(
                &self,
                _req: ApprovalRequest,
            ) -> orka_core::Result<ApprovalDecision> {
                Ok(ApprovalDecision::Denied {
                    reason: "test denial".into(),
                })
            }
        }

        let config = OsConfig {
            permission_level: "admin".into(),
            sudo: SudoConfig {
                enabled: true,
                allowed_commands: vec!["echo".into()],
                require_confirmation: true,
                ..SudoConfig::default()
            },
            ..OsConfig::default()
        };
        let skill = ShellExecSkill::new(
            Arc::new(PermissionGuard::new(&config)),
            &config,
            Arc::new(DenyChannel),
        );
        let mut args = HashMap::new();
        args.insert("command".into(), serde_json::json!("echo"));
        args.insert("args".into(), serde_json::json!(["hi"]));
        args.insert("sudo".into(), serde_json::json!(true));
        let err = skill.execute(SkillInput::new(args)).await.unwrap_err();
        assert!(err.to_string().contains("denied"));
    }

    #[test]
    fn schema_includes_sudo_for_admin() {
        use crate::approval::AutoApproveChannel;
        use orka_core::config::{OsConfig, SudoConfig};
        let config = OsConfig {
            permission_level: "admin".into(),
            sudo: SudoConfig {
                enabled: true,
                ..SudoConfig::default()
            },
            ..OsConfig::default()
        };
        let skill = ShellExecSkill::new(
            Arc::new(PermissionGuard::new(&config)),
            &config,
            Arc::new(AutoApproveChannel),
        );
        let schema = skill.schema();
        assert!(schema.parameters["properties"]["sudo"].is_object());
    }

    #[test]
    fn schema_hides_sudo_for_non_admin() {
        use crate::approval::AutoApproveChannel;
        use orka_core::config::{OsConfig, SudoConfig};
        let config = OsConfig {
            permission_level: "execute".into(),
            sudo: SudoConfig {
                enabled: true,
                ..SudoConfig::default()
            },
            ..OsConfig::default()
        };
        let skill = ShellExecSkill::new(
            Arc::new(PermissionGuard::new(&config)),
            &config,
            Arc::new(AutoApproveChannel),
        );
        let schema = skill.schema();
        assert!(schema.parameters["properties"]["sudo"].is_null());
    }
}
