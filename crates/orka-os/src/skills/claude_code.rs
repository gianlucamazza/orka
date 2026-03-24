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
    system_prompt: Option<String>,
    allowed_tools: Vec<String>,
    inject_context: bool,
}

impl ClaudeCodeSkill {
    /// Create a new `ClaudeCodeSkill` from the OS config.
    pub fn new(_config: &orka_core::config::ClaudeCodeConfig) -> Self {
        // Use default values - ClaudeCodeConfig is minimal now
        Self {
            working_dir: None,
            model: None,
            max_turns: None,
            timeout_secs: 300, // 5 minutes default
            system_prompt: None,
            allowed_tools: vec![],
            inject_context: false,
        }
    }
}

#[async_trait]
impl Skill for ClaudeCodeSkill {
    fn name(&self) -> &str {
        "claude_code"
    }

    fn category(&self) -> &str {
        "shell"
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
                    "description": "Imperative description of the coding task. Be specific: mention file paths, \
                                    the language/framework, constraints, and expected outcome."
                },
                "context": {
                    "type": "string",
                    "description": "Additional context: relevant files, recent changes, architectural constraints, \
                                    or background that Claude Code needs to understand the task."
                },
                "verification": {
                    "type": "string",
                    "description": "Command to run after completing the task to verify correctness \
                                    (e.g. 'cargo test -p my-crate', 'npm test', 'python -m pytest'). \
                                    Claude Code will run this and report the outcome."
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
        let verification = input.args.get("verification").and_then(|v| v.as_str());

        let prompt = build_prompt(task, context, verification, &input, self.inject_context);

        let working_dir = input
            .args
            .get("working_dir")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .or_else(|| self.working_dir.clone())
            .or_else(|| {
                input
                    .context
                    .as_ref()
                    .and_then(|c| c.user_cwd.as_deref())
                    .map(PathBuf::from)
            });

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
        if let Some(sp) = &self.system_prompt {
            cmd.arg("--append-system-prompt").arg(sp);
        }
        if !self.allowed_tools.is_empty() {
            cmd.arg("--allowedTools").arg(self.allowed_tools.join(","));
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
            Ok(Err(e)) => Err(Error::SkillCategorized {
                message: format!("claude execution failed: {e}"),
                category: ErrorCategory::Unknown,
            }),
            Err(_) => Err(Error::SkillCategorized {
                message: format!("claude timed out after {} seconds", self.timeout_secs),
                category: ErrorCategory::Timeout,
            }),
        }
    }
}

/// Build the structured prompt for Claude Code following delegation best practices:
/// clear sections, explicit requirements, and optional verification criteria.
fn build_prompt(
    task: &str,
    context: Option<&str>,
    verification: Option<&str>,
    input: &SkillInput,
    inject_context: bool,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    parts.push(format!("## Task\n{task}"));

    if let Some(ctx) = context {
        parts.push(format!("## Context\n{ctx}"));
    }

    if inject_context && let Some(workspace) = workspace_info(input) {
        parts.push(format!("## Workspace\n{workspace}"));
    }

    parts.push(
        "## Requirements\n\
         - Act autonomously: read the relevant files, make the changes, and verify the result.\n\
         - Follow existing code conventions (style, error handling, test patterns) found in the project.\n\
         - After completing the task, report concisely: what changed, why, and the outcome of any checks."
            .to_string(),
    );

    if let Some(v) = verification {
        parts.push(format!(
            "## Verification\nRun the following command to confirm the task is complete:\n```\n{v}\n```\nReport whether it passed or failed."
        ));
    }

    parts.join("\n\n")
}

/// Extract workspace metadata from the skill input context.
fn workspace_info(input: &SkillInput) -> Option<String> {
    let ctx = input.context.as_ref()?;
    let cwd = ctx.user_cwd.as_deref()?;
    Some(format!("Working directory: {cwd}"))
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
    use orka_core::config::ClaudeCodeConfig;

    use super::*;

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
    fn schema_has_verification_field() {
        let skill = make_skill();
        let schema = skill.schema();
        assert!(schema.parameters["properties"]["verification"].is_object());
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

    #[test]
    fn build_prompt_task_only() {
        let input = SkillInput::new(std::collections::HashMap::new());
        let prompt = build_prompt("Fix the bug", None, None, &input, false);
        assert!(prompt.contains("## Task\nFix the bug"));
        assert!(prompt.contains("## Requirements"));
        assert!(!prompt.contains("## Context"));
        assert!(!prompt.contains("## Verification"));
    }

    #[test]
    fn build_prompt_with_context_and_verification() {
        let input = SkillInput::new(std::collections::HashMap::new());
        let prompt = build_prompt(
            "Add retry logic",
            Some("See src/client.rs"),
            Some("cargo test -p orka-http"),
            &input,
            false,
        );
        assert!(prompt.contains("## Context\nSee src/client.rs"));
        assert!(prompt.contains("## Verification"));
        assert!(prompt.contains("cargo test -p orka-http"));
    }

    #[tokio::test]
    async fn missing_task_errors() {
        use std::collections::HashMap;
        let skill = make_skill();
        let input = SkillInput::new(HashMap::new());
        assert!(skill.execute(input).await.is_err());
    }

    #[test]
    fn build_prompt_inject_context_no_cwd_omits_workspace() {
        // inject_context=true but no SkillContext (SkillContext is #[non_exhaustive],
        // so it can only be constructed inside orka-core; full cwd injection is covered
        // by integration tests).
        let input = SkillInput::new(std::collections::HashMap::new());
        let prompt = build_prompt("Fix bug", None, None, &input, true);
        assert!(prompt.contains("## Task"));
        assert!(prompt.contains("## Requirements"));
        assert!(!prompt.contains("## Workspace"));
    }
}
