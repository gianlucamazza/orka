mod stream;

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use orka_core::{
    Error, ErrorCategory, Result, SkillInput, SkillOutput, SkillSchema, traits::Skill,
};
use stream::DelegateEvent;
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::debug;

use crate::config::{
    ClaudeCodeConfig, CodexConfig, CodingConfig, CodingProvider, CodingSelectionPolicy,
    OpenCodeConfig, OsConfig, SandboxMode,
};

const CODING_CATEGORY: &str = "coding";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackendKind {
    ClaudeCode,
    Codex,
    OpenCode,
}

impl BackendKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
        }
    }
}

struct CodingRequest {
    task: String,
    context: Option<String>,
    verification: Option<String>,
    working_dir: Option<PathBuf>,
}

impl CodingRequest {
    fn parse(input: &SkillInput, config: &CodingConfig) -> Result<Self> {
        let task = input
            .args
            .get("task")
            .and_then(|v| v.as_str())
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'task' argument".into(),
                category: ErrorCategory::Input,
            })?;

        let verification = input
            .args
            .get("verification")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        if config.require_verification && verification.is_none() {
            return Err(Error::SkillCategorized {
                message: "missing 'verification' argument: os.coding.require_verification = true"
                    .into(),
                category: ErrorCategory::Input,
            });
        }

        let working_dir = if config.allow_working_dir_override {
            input
                .args
                .get("working_dir")
                .and_then(|v| v.as_str())
                .map(PathBuf::from)
        } else {
            None
        };

        Ok(Self {
            task,
            context: input
                .args
                .get("context")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            verification,
            working_dir,
        })
    }
}

#[async_trait]
trait CodeDelegateBackend: Send + Sync {
    fn kind(&self) -> BackendKind;
    fn is_enabled(&self) -> bool;
    fn executable_name(&self) -> &'static str;
    fn executable_override(&self) -> Option<&Path>;
    fn timeout_secs(&self) -> u64;
    fn allow_file_modifications(&self) -> bool;
    fn allow_command_execution(&self) -> bool;
    async fn run(
        &self,
        request: &CodingRequest,
        input: &SkillInput,
        coding: &CodingConfig,
    ) -> Result<SkillOutput>;
}

fn schema() -> SkillSchema {
    SkillSchema::new(serde_json::json!({
        "type": "object",
        "properties": {
            "task": {
                "type": "string",
                "description": "Imperative description of the coding task, including files, constraints, and expected outcome."
            },
            "context": {
                "type": "string",
                "description": "Additional architectural or repository context for the delegated coding agent."
            },
            "verification": {
                "type": "string",
                "description": "Command that must be run after the implementation to verify correctness."
            },
            "working_dir": {
                "type": "string",
                "description": "Optional working directory override. Only honored when os.coding.allow_working_dir_override = true."
            }
        },
        "required": ["task"]
    }))
}

/// Routing entrypoint that selects the configured coding backend at runtime.
pub struct CodingDelegateSkill {
    coding: CodingConfig,
    claude: ClaudeCodeBackend,
    codex: CodexBackend,
    opencode: OpenCodeBackend,
}

impl CodingDelegateSkill {
    /// Create a routing skill from the full OS configuration.
    pub fn new(config: &OsConfig) -> Self {
        Self {
            coding: config.coding.clone(),
            claude: ClaudeCodeBackend::new(&config.coding.providers.claude_code),
            codex: CodexBackend::new(&config.coding.providers.codex),
            opencode: OpenCodeBackend::new(&config.coding.providers.opencode),
        }
    }

    fn select_backend(&self) -> Result<&dyn CodeDelegateBackend> {
        let claude_enabled = self.claude.is_enabled();
        let codex_enabled = self.codex.is_enabled();
        let opencode_enabled = self.opencode.is_enabled();

        let chosen = match self.coding.default_provider {
            CodingProvider::ClaudeCode => {
                if claude_enabled {
                    Some(&self.claude as &dyn CodeDelegateBackend)
                } else {
                    None
                }
            }
            CodingProvider::Codex => {
                if codex_enabled {
                    Some(&self.codex as &dyn CodeDelegateBackend)
                } else {
                    None
                }
            }
            CodingProvider::OpenCode => {
                if opencode_enabled {
                    Some(&self.opencode as &dyn CodeDelegateBackend)
                } else {
                    None
                }
            }
            CodingProvider::Auto => match self.coding.selection_policy {
                // Availability / PreferClaude: first available in claude → codex → opencode order.
                CodingSelectionPolicy::Availability | CodingSelectionPolicy::PreferClaude => {
                    if claude_enabled {
                        Some(&self.claude as &dyn CodeDelegateBackend)
                    } else if codex_enabled {
                        Some(&self.codex as &dyn CodeDelegateBackend)
                    } else if opencode_enabled {
                        Some(&self.opencode as &dyn CodeDelegateBackend)
                    } else {
                        None
                    }
                }
                // PreferCodex: codex → claude → opencode
                CodingSelectionPolicy::PreferCodex => {
                    if codex_enabled {
                        Some(&self.codex as &dyn CodeDelegateBackend)
                    } else if claude_enabled {
                        Some(&self.claude as &dyn CodeDelegateBackend)
                    } else if opencode_enabled {
                        Some(&self.opencode as &dyn CodeDelegateBackend)
                    } else {
                        None
                    }
                }
                // PreferOpenCode: opencode → claude → codex
                CodingSelectionPolicy::PreferOpenCode => {
                    if opencode_enabled {
                        Some(&self.opencode as &dyn CodeDelegateBackend)
                    } else if claude_enabled {
                        Some(&self.claude as &dyn CodeDelegateBackend)
                    } else if codex_enabled {
                        Some(&self.codex as &dyn CodeDelegateBackend)
                    } else {
                        None
                    }
                }
            },
        };

        chosen.ok_or_else(|| Error::SkillCategorized {
            message: "no coding backend available: enable a provider under os.coding.providers"
                .into(),
            category: ErrorCategory::Environmental,
        })
    }
}

#[async_trait]
impl Skill for CodingDelegateSkill {
    fn name(&self) -> &'static str {
        "coding_delegate"
    }

    fn category(&self) -> &'static str {
        CODING_CATEGORY
    }

    fn description(&self) -> &'static str {
        "Delegate a complete coding task through Orka's coding router. Orka selects the configured provider, executes the task, and returns a normalized summary."
    }

    fn budget_cost(&self) -> f32 {
        0.0
    }

    fn schema(&self) -> SkillSchema {
        schema()
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let request = CodingRequest::parse(&input, &self.coding)?;
        let backend = self.select_backend()?;
        backend.run(&request, &input, &self.coding).await
    }
}

struct ClaudeCodeBackend {
    config: ClaudeCodeConfig,
}

impl ClaudeCodeBackend {
    fn new(config: &ClaudeCodeConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }
}

#[async_trait]
impl CodeDelegateBackend for ClaudeCodeBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::ClaudeCode
    }

    fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    fn executable_name(&self) -> &'static str {
        "claude"
    }

    fn executable_override(&self) -> Option<&Path> {
        self.config.executable_path.as_deref()
    }

    fn timeout_secs(&self) -> u64 {
        self.config.timeout_secs
    }

    fn allow_file_modifications(&self) -> bool {
        self.config.allow_file_modifications
    }

    fn allow_command_execution(&self) -> bool {
        self.config.allow_command_execution
    }

    async fn run(
        &self,
        request: &CodingRequest,
        input: &SkillInput,
        coding: &CodingConfig,
    ) -> Result<SkillOutput> {
        let prompt = build_prompt(self, request, input, coding);
        let streaming = input
            .context
            .as_ref()
            .and_then(|c| c.progress_tx.as_ref())
            .is_some();

        let mut cmd = tokio::process::Command::new(executable(self));
        cmd.arg("--bare")
            .arg("--print")
            .arg("--output-format")
            // Use stream-json when a progress channel is present so we can
            // emit intermediate events; fall back to json for compatibility.
            .arg(if streaming { "stream-json" } else { "json" })
            .arg("-p")
            .arg(prompt);

        if let Some(model) = &self.config.model {
            cmd.arg("--model").arg(model);
        }
        if let Some(max_turns) = self.config.max_turns {
            cmd.arg("--max-turns").arg(max_turns.to_string());
        }
        if let Some(system_prompt) = &self.config.append_system_prompt {
            cmd.arg("--append-system-prompt").arg(system_prompt);
        }
        if !self.config.allowed_tools.is_empty() {
            cmd.arg("--allowedTools")
                .arg(self.config.allowed_tools.join(","));
        }

        apply_working_dir(self, request, input, coding, &mut cmd);
        execute_command_streaming(
            self,
            cmd,
            stream::parse_claude_stream_line,
            parse_claude_output,
            input.context.as_ref().and_then(|c| c.progress_tx.clone()),
            input
                .context
                .as_ref()
                .and_then(|c| c.cancellation_token.clone()),
        )
        .await
    }
}

struct CodexBackend {
    config: CodexConfig,
}

impl CodexBackend {
    fn new(config: &CodexConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }
}

#[async_trait]
impl CodeDelegateBackend for CodexBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Codex
    }

    fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    fn executable_name(&self) -> &'static str {
        "codex"
    }

    fn executable_override(&self) -> Option<&Path> {
        self.config.executable_path.as_deref()
    }

    fn timeout_secs(&self) -> u64 {
        self.config.timeout_secs
    }

    fn allow_file_modifications(&self) -> bool {
        self.config.allow_file_modifications
    }

    fn allow_command_execution(&self) -> bool {
        self.config.allow_command_execution
    }

    async fn run(
        &self,
        request: &CodingRequest,
        input: &SkillInput,
        coding: &CodingConfig,
    ) -> Result<SkillOutput> {
        let prompt = build_prompt(self, request, input, coding);
        let mut cmd = tokio::process::Command::new(executable(self));
        if let Some(policy) = self.config.approval_policy {
            cmd.arg("-a").arg(policy.to_string());
        }
        cmd.arg("exec")
            .arg("--skip-git-repo-check")
            .arg("--json")
            .arg("--ephemeral");

        if let Some(model) = &self.config.model {
            cmd.arg("--model").arg(model);
        }
        cmd.arg("--sandbox")
            .arg(effective_codex_sandbox(&self.config).to_string());

        apply_working_dir(self, request, input, coding, &mut cmd);
        cmd.arg(prompt);

        execute_command_streaming(
            self,
            cmd,
            stream::parse_codex_stream_line,
            parse_codex_output,
            input.context.as_ref().and_then(|c| c.progress_tx.clone()),
            input
                .context
                .as_ref()
                .and_then(|c| c.cancellation_token.clone()),
        )
        .await
    }
}

struct OpenCodeBackend {
    config: OpenCodeConfig,
}

impl OpenCodeBackend {
    fn new(config: &OpenCodeConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }
}

#[async_trait]
impl CodeDelegateBackend for OpenCodeBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::OpenCode
    }

    fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    fn executable_name(&self) -> &'static str {
        "opencode"
    }

    fn executable_override(&self) -> Option<&Path> {
        self.config.executable_path.as_deref()
    }

    fn timeout_secs(&self) -> u64 {
        self.config.timeout_secs
    }

    fn allow_file_modifications(&self) -> bool {
        self.config.allow_file_modifications
    }

    fn allow_command_execution(&self) -> bool {
        self.config.allow_command_execution
    }

    async fn run(
        &self,
        request: &CodingRequest,
        input: &SkillInput,
        coding: &CodingConfig,
    ) -> Result<SkillOutput> {
        let prompt = build_prompt(self, request, input, coding);
        let mut cmd = tokio::process::Command::new(executable(self));
        cmd.arg("run").arg("--format").arg("json");

        if let Some(model) = &self.config.model {
            cmd.arg("--model").arg(model);
        }
        if let Some(agent) = &self.config.agent {
            cmd.arg("--agent").arg(agent);
        }
        if let Some(variant) = &self.config.variant {
            cmd.arg("--variant").arg(variant);
        }

        apply_working_dir(self, request, input, coding, &mut cmd);
        cmd.arg(prompt);

        execute_command_streaming(
            self,
            cmd,
            stream::parse_opencode_stream_line,
            parse_opencode_output,
            input.context.as_ref().and_then(|c| c.progress_tx.clone()),
            input
                .context
                .as_ref()
                .and_then(|c| c.cancellation_token.clone()),
        )
        .await
    }
}

fn executable(backend: &dyn CodeDelegateBackend) -> String {
    backend.executable_override().map_or_else(
        || backend.executable_name().to_string(),
        |path| path.to_string_lossy().into_owned(),
    )
}

fn apply_working_dir(
    backend: &dyn CodeDelegateBackend,
    request: &CodingRequest,
    input: &SkillInput,
    coding: &CodingConfig,
    cmd: &mut tokio::process::Command,
) {
    let working_dir = resolve_working_dir(request, input, coding);
    if let Some(dir) = &working_dir {
        cmd.current_dir(dir);
        if backend.kind() == BackendKind::Codex {
            cmd.arg("--cd").arg(dir);
        }
    }
}

fn resolve_working_dir(
    request: &CodingRequest,
    input: &SkillInput,
    coding: &CodingConfig,
) -> Option<PathBuf> {
    // 1. Explicit LLM override (only when permitted by config)
    if coding.allow_working_dir_override && request.working_dir.is_some() {
        return request.working_dir.clone();
    }

    // 2. Active worktree context — propagated automatically by the node runner
    //    after a successful `git_worktree_create` call.
    if let Some(wt) = input
        .context
        .as_ref()
        .and_then(|c| c.worktree_cwd.as_deref())
    {
        return Some(PathBuf::from(wt));
    }

    // 3. User's CWD (from CLI `workspace:cwd` metadata)
    input
        .context
        .as_ref()
        .and_then(|ctx| ctx.user_cwd.as_deref())
        .map(PathBuf::from)
}

/// Execute a coding-delegate command with line-by-line stdout streaming.
///
/// Streams intermediate [`DelegateEvent`]s through `progress_tx` (if set)
/// and returns a final [`SkillOutput`] once the process exits.  Supports
/// cooperative cancellation via `cancel`.
#[allow(clippy::too_many_lines)]
async fn execute_command_streaming(
    backend: &dyn CodeDelegateBackend,
    mut cmd: tokio::process::Command,
    line_parser: fn(&str) -> Option<DelegateEvent>,
    fallback_parser: fn(&str) -> String,
    progress_tx: Option<tokio::sync::mpsc::UnboundedSender<serde_json::Value>>,
    cancel: Option<tokio_util::sync::CancellationToken>,
) -> Result<SkillOutput> {
    use std::process::Stdio;

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    debug!(
        backend = backend.kind().as_str(),
        timeout_secs = backend.timeout_secs(),
        streaming = progress_tx.is_some(),
        "delegating coding task"
    );

    let start = std::time::Instant::now();
    let mut child = cmd.spawn().map_err(|e| spawn_error(backend, &e))?;

    let stdout = child.stdout.take().ok_or_else(|| Error::SkillCategorized {
        message: "stdout not available".into(),
        category: ErrorCategory::Unknown,
    })?;
    let mut stderr_handle = child.stderr.take().ok_or_else(|| Error::SkillCategorized {
        message: "stderr not available".into(),
        category: ErrorCategory::Unknown,
    })?;
    let mut lines = BufReader::new(stdout).lines();

    let timeout = std::time::Duration::from_secs(backend.timeout_secs());
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);

    // When no cancellation token is provided, this future never resolves.
    let cancel_fut = async {
        match &cancel {
            Some(t) => t.cancelled().await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::pin!(cancel_fut);

    let mut accumulated: Vec<String> = Vec::new();
    let mut final_text: Option<String> = None;

    loop {
        tokio::select! {
            biased;

            () = &mut deadline => {
                child.kill().await.ok();
                return Err(Error::SkillCategorized {
                    message: format!(
                        "{} timed out after {} seconds",
                        backend.kind().as_str(),
                        backend.timeout_secs()
                    ),
                    category: ErrorCategory::Timeout,
                });
            }

            () = &mut cancel_fut => {
                child.kill().await.ok();
                return Err(Error::SkillCategorized {
                    message: format!("{} cancelled", backend.kind().as_str()),
                    category: ErrorCategory::Timeout,
                });
            }

            line_result = lines.next_line() => {
                match line_result {
                    Ok(Some(line)) => {
                        if let Some(event) = line_parser(&line) {
                            // Capture the final result text when we see it.
                            if let DelegateEvent::Result { ref text } = event {
                                final_text = Some(text.clone());
                            }
                            // Forward to progress channel (best-effort).
                            if let Some(ref tx) = progress_tx
                                && let Ok(val) = serde_json::to_value(&event) {
                                    tx.send(val).ok();
                                }
                        }
                        accumulated.push(line);
                    }
                    // Stream ended (EOF or broken pipe).
                    Ok(None) | Err(_) => break,
                }
            }
        }
    }

    let status = child.wait().await.map_err(|e| Error::SkillCategorized {
        message: format!("{} wait failed: {e}", backend.kind().as_str()),
        category: ErrorCategory::Unknown,
    })?;

    // Collect stderr for error messages.
    let mut stderr_buf = Vec::new();
    tokio::io::copy(&mut stderr_handle, &mut stderr_buf)
        .await
        .ok();
    let stderr = String::from_utf8_lossy(&stderr_buf);

    if !status.success() {
        return Err(Error::SkillCategorized {
            message: format!(
                "{} exited with status {:?}: {}",
                backend.kind().as_str(),
                status.code(),
                stderr.trim()
            ),
            category: ErrorCategory::Unknown,
        });
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    let result_text = final_text.unwrap_or_else(|| {
        // Fall back to the legacy full-output parser if no structured Result
        // event was emitted (e.g. backend used non-streaming format).
        let raw = accumulated.join("\n");
        fallback_parser(&raw)
    });

    Ok(SkillOutput::new(serde_json::json!({
        "backend": backend.kind().as_str(),
        "result": result_text,
        "duration_ms": duration_ms,
    })))
}

fn spawn_error(backend: &dyn CodeDelegateBackend, e: &std::io::Error) -> Error {
    let category = match e.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
            ErrorCategory::Environmental
        }
        _ => ErrorCategory::Unknown,
    };
    Error::SkillCategorized {
        message: format!("failed to spawn '{}': {e}", executable(backend)),
        category,
    }
}

fn build_prompt(
    backend: &dyn CodeDelegateBackend,
    request: &CodingRequest,
    input: &SkillInput,
    coding: &CodingConfig,
) -> String {
    let mut parts = vec![format!("## Task\n{}", request.task)];

    if let Some(context) = &request.context {
        parts.push(format!("## Context\n{context}"));
    }

    if coding.inject_workspace_context
        && let Some(workspace) = workspace_info(input)
    {
        parts.push(format!("## Workspace\n{workspace}"));
    }

    let mut constraints = Vec::new();
    constraints.push("Act autonomously: inspect the repository, make only the necessary changes, and return a concise implementation summary.".to_string());
    constraints.push("Preserve existing architecture and conventions; do not introduce workaround layers or parallel abstractions.".to_string());
    if backend.allow_file_modifications() {
        constraints
            .push("File modifications are allowed when needed to complete the task.".to_string());
    } else {
        constraints.push(
            "Do not modify files. Limit yourself to analysis and recommendations.".to_string(),
        );
    }
    if backend.allow_command_execution() {
        constraints.push(
            "Command execution is allowed when it materially verifies or informs the change."
                .to_string(),
        );
    } else {
        constraints.push("Do not execute commands beyond safe repository inspection.".to_string());
    }
    parts.push(format!("## Constraints\n- {}", constraints.join("\n- ")));

    if let Some(verification) = &request.verification {
        parts.push(format!(
            "## Verification\nRun this command after completing the task and report the result:\n```\n{verification}\n```"
        ));
    }

    parts.join("\n\n")
}

fn workspace_info(input: &SkillInput) -> Option<String> {
    let ctx = input.context.as_ref()?;
    let cwd = ctx.user_cwd.as_deref()?;
    Some(format!("Working directory: {cwd}"))
}

fn parse_claude_output(raw: &str) -> String {
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

    raw.trim().to_string()
}

fn parse_codex_output(raw: &str) -> String {
    let mut candidates = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed)
            && let Some(text) = extract_codex_text(&val)
        {
            candidates.push(text);
        }
    }

    candidates
        .into_iter()
        .rev()
        .find(|text| !text.trim().is_empty())
        .unwrap_or_else(|| raw.trim().to_string())
}

fn extract_codex_text(val: &serde_json::Value) -> Option<String> {
    for key in ["result", "last_message", "content", "text", "message"] {
        if let Some(text) = val.get(key).and_then(value_to_text) {
            return Some(text);
        }
    }

    if let Some(event) = val.get("event") {
        for key in ["result", "last_message", "content", "text", "message"] {
            if let Some(text) = event.get(key).and_then(value_to_text) {
                return Some(text);
            }
        }
    }

    None
}

fn value_to_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Array(items) => {
            let joined = items
                .iter()
                .filter_map(value_to_text)
                .collect::<Vec<_>>()
                .join("\n");
            if joined.is_empty() {
                None
            } else {
                Some(joined)
            }
        }
        serde_json::Value::Object(map) => map
            .get("text")
            .and_then(value_to_text)
            .or_else(|| map.get("content").and_then(value_to_text)),
        _ => None,
    }
}

/// Extract the final assistant text from accumulated `opencode run --format
/// json` output.
///
/// `OpenCode` emits `{"type":"text","part":{"text":"...",...}}` lines during
/// the run. There is no dedicated result line, so the fallback parser scans
/// backwards for the last non-empty `text` event.
fn parse_opencode_output(raw: &str) -> String {
    for line in raw.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed)
            && val.get("type").and_then(|t| t.as_str()) == Some("text")
            && let Some(text) = val
                .get("part")
                .and_then(|p| p.get("text"))
                .and_then(|t| t.as_str())
            && !text.trim().is_empty()
        {
            return text.to_string();
        }
    }
    raw.trim().to_string()
}

fn effective_codex_sandbox(config: &CodexConfig) -> SandboxMode {
    config.sandbox_mode.unwrap_or(
        if config.allow_file_modifications || config.allow_command_execution {
            SandboxMode::WorkspaceWrite
        } else {
            SandboxMode::ReadOnly
        },
    )
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
    use super::*;

    fn base_os_config() -> OsConfig {
        let mut config = OsConfig::default();
        config.coding.enabled = true;
        config.coding.default_provider = CodingProvider::Auto;
        config.coding.providers.claude_code.enabled = true;
        config.coding.providers.codex.enabled = true;
        config
    }

    #[test]
    fn schema_requires_task() {
        let skill = CodingDelegateSkill::new(&base_os_config());
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
    fn parse_codex_output_prefers_final_message() {
        let raw = r#"{"type":"progress","text":"working"}"#.to_string()
            + "\n"
            + r#"{"type":"result","message":"done"}"#;
        assert_eq!(parse_codex_output(&raw), "done");
    }

    #[test]
    fn prompt_contains_no_workaround_constraint() {
        let config = base_os_config();
        let backend = ClaudeCodeBackend::new(&config.coding.providers.claude_code);
        let request = CodingRequest {
            task: "Refactor the module".into(),
            context: None,
            verification: None,
            working_dir: None,
        };
        let prompt = build_prompt(
            &backend,
            &request,
            &SkillInput::new(std::collections::HashMap::new()),
            &config.coding,
        );
        assert!(prompt.contains("do not introduce workaround layers"));
    }

    #[test]
    fn auto_selection_prefers_configured_policy() {
        let mut config = base_os_config();
        config.coding.selection_policy = CodingSelectionPolicy::PreferCodex;
        let skill = CodingDelegateSkill::new(&config);
        let selected = skill.select_backend().unwrap();
        assert_eq!(selected.kind(), BackendKind::Codex);
    }

    #[test]
    fn direct_backend_uses_explicit_default() {
        let mut config = base_os_config();
        config.coding.default_provider = CodingProvider::ClaudeCode;
        config.coding.providers.codex.enabled = false;
        let skill = CodingDelegateSkill::new(&config);
        let selected = skill.select_backend().unwrap();
        assert_eq!(selected.kind(), BackendKind::ClaudeCode);
    }

    #[test]
    fn no_backend_available_errors() {
        let mut config = base_os_config();
        config.coding.providers.claude_code.enabled = false;
        config.coding.providers.codex.enabled = false;
        config.coding.providers.opencode.enabled = false;
        let skill = CodingDelegateSkill::new(&config);
        assert!(skill.select_backend().is_err());
    }

    #[test]
    fn opencode_direct_backend_selection() {
        let mut config = base_os_config();
        config.coding.default_provider = CodingProvider::OpenCode;
        config.coding.providers.opencode.enabled = true;
        let skill = CodingDelegateSkill::new(&config);
        let selected = skill.select_backend().unwrap();
        assert_eq!(selected.kind(), BackendKind::OpenCode);
    }

    #[test]
    fn prefer_opencode_policy_selects_opencode_first() {
        let mut config = base_os_config();
        config.coding.selection_policy = CodingSelectionPolicy::PreferOpenCode;
        config.coding.providers.opencode.enabled = true;
        let skill = CodingDelegateSkill::new(&config);
        let selected = skill.select_backend().unwrap();
        assert_eq!(selected.kind(), BackendKind::OpenCode);
    }

    #[test]
    fn prefer_opencode_falls_back_to_claude_when_opencode_disabled() {
        let mut config = base_os_config();
        config.coding.selection_policy = CodingSelectionPolicy::PreferOpenCode;
        config.coding.providers.opencode.enabled = false;
        let skill = CodingDelegateSkill::new(&config);
        let selected = skill.select_backend().unwrap();
        assert_eq!(selected.kind(), BackendKind::ClaudeCode);
    }

    #[test]
    fn availability_policy_uses_opencode_as_last_fallback() {
        let mut config = base_os_config();
        config.coding.providers.claude_code.enabled = false;
        config.coding.providers.codex.enabled = false;
        config.coding.providers.opencode.enabled = true;
        let skill = CodingDelegateSkill::new(&config);
        let selected = skill.select_backend().unwrap();
        assert_eq!(selected.kind(), BackendKind::OpenCode);
    }

    #[test]
    fn parse_opencode_output_extracts_last_text() {
        let raw = concat!(
            r#"{"type":"step_start","part":{"type":"step-start"}}"#,
            "\n",
            r#"{"type":"text","part":{"type":"text","text":"first response","time":{}}}"#,
            "\n",
            r#"{"type":"text","part":{"type":"text","text":"final answer","time":{}}}"#,
            "\n",
            r#"{"type":"step_finish","part":{"type":"step-finish","reason":"stop"}}"#,
        );
        assert_eq!(parse_opencode_output(raw), "final answer");
    }
}
