use std::path::PathBuf;

use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Error, ErrorCategory, Result, SkillInput, SkillOutput, SkillSchema};
use tracing::debug;

/// Skill that delegates a coding task to `claude --print`, running Claude Code as a subprocess.
///
/// Useful for tasks that benefit from Claude Code's multi-step reasoning and tool use
/// (reading files, editing code, running tests) without wiring each individual tool.
pub struct ClaudeCodeSkill {
    working_dir: Option<PathBuf>,
    model: Option<String>,
    max_turns: Option<u32>,
    timeout_secs: u64,
}

impl ClaudeCodeSkill {
    /// Create a new `ClaudeCodeSkill` from the OS config.
    pub fn new(config: &orka_core::config::ClaudeCodeConfig) -> Self {
        Self {
            working_dir: config.working_dir.as_deref().map(PathBuf::from),
            model: config.model.clone(),
            max_turns: config.max_turns,
            timeout_secs: config.timeout_secs,
        }
    }
}

#[async_trait]
impl Skill for ClaudeCodeSkill {
    fn name(&self) -> &str {
        "claude_code"
    }

    fn description(&self) -> &str {
        "Delegate a complete coding task to Claude Code. Claude Code will autonomously read files, \
         make edits, run commands, and return a summary of what was done. Use for complex, \
         multi-step coding tasks that require understanding the codebase."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Description of the coding task to perform"
                },
                "context": {
                    "type": "string",
                    "description": "Additional context: relevant files, constraints, or background"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Working directory override (defaults to configured directory)"
                }
            },
            "required": ["task"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let task = input
            .args
            .get("task")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'task' argument".into(),
                category: ErrorCategory::Input,
            })?;

        let context = input.args.get("context").and_then(|v| v.as_str());

        let prompt = if let Some(ctx) = context {
            format!("{task}\n\nContext:\n{ctx}")
        } else {
            task.to_string()
        };

        let working_dir = input
            .args
            .get("working_dir")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .or_else(|| self.working_dir.clone());

        let mut cmd = tokio::process::Command::new("claude");
        cmd.arg("--print");
        cmd.arg("--output-format").arg("json");
        cmd.arg("-p").arg(&prompt);

        if let Some(model) = &self.model {
            cmd.arg("--model").arg(model);
        }
        if let Some(max_turns) = self.max_turns {
            cmd.arg("--max-turns").arg(max_turns.to_string());
        }
        if let Some(dir) = &working_dir {
            cmd.current_dir(dir);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        debug!(
            task,
            timeout_secs = self.timeout_secs,
            "claude_code delegating task"
        );

        let start = std::time::Instant::now();

        let child = cmd.spawn().map_err(|e| {
            let category = match e.kind() {
                std::io::ErrorKind::NotFound => ErrorCategory::Environmental,
                std::io::ErrorKind::PermissionDenied => ErrorCategory::Environmental,
                _ => ErrorCategory::Unknown,
            };
            Error::SkillCategorized {
                message: format!("failed to spawn 'claude': {e} — is Claude Code installed?"),
                category,
            }
        })?;

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(self.timeout_secs),
            child.wait_with_output(),
        )
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                if !output.status.success() {
                    return Err(Error::SkillCategorized {
                        message: format!(
                            "claude exited with status {:?}: {}",
                            output.status.code(),
                            stderr.trim()
                        ),
                        category: ErrorCategory::Unknown,
                    });
                }

                // Try to parse JSON output and extract the result field.
                // Claude Code --output-format json produces:
                // {"type":"result","subtype":"success","result":"...","is_error":false,...}
                let result_text = parse_claude_output(&stdout);

                Ok(SkillOutput::new(serde_json::json!({
                    "result": result_text,
                    "duration_ms": duration_ms,
                })))
            }
            Ok(Err(e)) => Err(Error::Skill(format!("claude execution failed: {e}"))),
            Err(_) => Err(Error::SkillCategorized {
                message: format!("claude timed out after {} seconds", self.timeout_secs),
                category: ErrorCategory::Timeout,
            }),
        }
    }
}

/// Extract the result text from `claude --output-format json` output.
///
/// Falls back to the raw stdout if JSON parsing fails or the expected field is missing.
fn parse_claude_output(raw: &str) -> String {
    // Claude Code outputs one JSON object per line in stream mode, or a single object.
    // Look for a line with `"type":"result"` and extract `.result`.
    for line in raw.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed)
            && val.get("type").and_then(|t| t.as_str()) == Some("result")
            && let Some(text) = val.get("result").and_then(|r| r.as_str())
        {
            return text.to_string();
        }
    }
    // Fall back: return the raw output
    raw.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use orka_core::config::ClaudeCodeConfig;

    fn make_skill() -> ClaudeCodeSkill {
        ClaudeCodeSkill::new(&ClaudeCodeConfig {
            enabled: "true".into(),
            timeout_secs: 30,
            ..ClaudeCodeConfig::default()
        })
    }

    #[test]
    fn schema_requires_task() {
        let skill = make_skill();
        let schema = skill.schema();
        assert_eq!(schema.parameters["required"][0], "task");
    }

    #[test]
    fn parse_claude_output_json() {
        let raw =
            r#"{"type":"result","subtype":"success","is_error":false,"result":"hello world"}"#;
        assert_eq!(parse_claude_output(raw), "hello world");
    }

    #[test]
    fn parse_claude_output_fallback() {
        let raw = "not json output";
        assert_eq!(parse_claude_output(raw), "not json output");
    }

    #[tokio::test]
    async fn missing_task_errors() {
        use std::collections::HashMap;
        let skill = make_skill();
        let input = SkillInput::new(HashMap::new());
        assert!(skill.execute(input).await.is_err());
    }
}
