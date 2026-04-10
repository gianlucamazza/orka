//! Skill input/output types, schema definitions, and soft-skill primitives.

use std::{collections::HashMap, sync::Arc};

use serde::{Deserialize, Serialize};

use super::envelope::MediaPayload;
use crate::traits::{EventSink, SecretManager};

/// Per-invocation budget constraints for a skill execution.
///
/// Fields are all optional — `None` means "no limit". Enforced by the
/// registry before and after `execute()`.
#[derive(Debug, Clone, Default)]
pub struct SkillBudget {
    /// Maximum wall-clock execution time in milliseconds.
    ///
    /// If the skill's measured duration exceeds this value the registry returns
    /// a [`ErrorCategory::Budget`] error after execution (not a hard timeout —
    /// use `AgentConfig::skill_timeout_secs` for hard cancellation).
    pub max_duration_ms: Option<u64>,
    /// Maximum allowed output size in bytes (serialized JSON).
    ///
    /// Prevents oversized skill outputs from flooding the LLM context window.
    pub max_output_bytes: Option<usize>,
}

/// Runtime context provided to a skill during execution.
#[derive(Clone)]
#[non_exhaustive]
pub struct SkillContext {
    /// Provides access to named secrets during skill execution.
    pub secrets: Arc<dyn SecretManager>,
    /// Optional sink for emitting domain events from within a skill.
    pub event_sink: Option<Arc<dyn EventSink>>,
    /// Optional per-invocation budget constraints.
    pub budget: Option<SkillBudget>,
    /// The user's working directory on the client machine, sent via
    /// `workspace:cwd` metadata. OS skills (e.g. `shell_exec`) should use
    /// this as their default CWD when the LLM does not explicitly supply
    /// one, so that commands run in the user's directory rather than the
    /// server process's working directory.
    pub user_cwd: Option<String>,
    /// Active git worktree path for this agent turn. When set, skills that
    /// operate on files or run commands (`shell_exec`, `coding_delegate`,
    /// `git_*`) should prefer this over `user_cwd` so the agent works
    /// inside the isolated worktree automatically, without needing to pass
    /// an explicit `path`/`cwd`/`working_dir` argument on every call.
    ///
    /// Set by the node runner when a `git_worktree_create` call succeeds;
    /// cleared when `git_worktree_remove` is called.
    pub worktree_cwd: Option<String>,
    /// Channel for streaming progress events from long-running skills.
    ///
    /// Used by `coding_delegate` to emit real-time [`DelegateEvent`]s.
    /// The payload is `serde_json::Value` to keep `orka-core` decoupled from
    /// skill-specific types.
    pub progress_tx: Option<tokio::sync::mpsc::UnboundedSender<serde_json::Value>>,
    /// Token checked by skills to support cooperative cancellation.
    pub cancellation_token: Option<tokio_util::sync::CancellationToken>,
}

impl std::fmt::Debug for SkillContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillContext").finish()
    }
}

impl SkillContext {
    /// Create a new skill context without a budget constraint.
    pub fn new(secrets: Arc<dyn SecretManager>, event_sink: Option<Arc<dyn EventSink>>) -> Self {
        Self {
            secrets,
            event_sink,
            budget: None,
            user_cwd: None,
            worktree_cwd: None,
            progress_tx: None,
            cancellation_token: None,
        }
    }

    /// Attach a [`SkillBudget`] to this context.
    #[must_use]
    pub fn with_budget(mut self, budget: SkillBudget) -> Self {
        self.budget = Some(budget);
        self
    }

    /// Set the user's working directory (from `workspace:cwd` envelope
    /// metadata).
    #[must_use]
    pub fn with_user_cwd(mut self, cwd: Option<String>) -> Self {
        self.user_cwd = cwd;
        self
    }

    /// Set the active git worktree path. Skills that operate on files or run
    /// commands will prefer this over `user_cwd` when set.
    #[must_use]
    pub fn with_worktree_cwd(mut self, cwd: Option<String>) -> Self {
        self.worktree_cwd = cwd;
        self
    }

    /// Attach a progress channel for streaming delegate events.
    #[must_use]
    pub fn with_progress(
        mut self,
        tx: tokio::sync::mpsc::UnboundedSender<serde_json::Value>,
    ) -> Self {
        self.progress_tx = Some(tx);
        self
    }

    /// Attach a cancellation token for cooperative cancellation.
    #[must_use]
    pub fn with_cancellation(mut self, token: tokio_util::sync::CancellationToken) -> Self {
        self.cancellation_token = Some(token);
        self
    }
}

/// Input passed to a skill invocation.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct SkillInput {
    /// Named arguments passed to the skill, keyed by parameter name.
    pub args: HashMap<String, serde_json::Value>,
    /// Runtime context injected by the worker before invocation.
    #[serde(skip)]
    #[schema(ignore)]
    pub context: Option<SkillContext>,
}

impl SkillInput {
    /// Create a new skill input with the given arguments.
    pub fn new(args: HashMap<String, serde_json::Value>) -> Self {
        Self {
            args,
            context: None,
        }
    }

    /// Set the skill context.
    #[must_use]
    pub fn with_context(mut self, context: SkillContext) -> Self {
        self.context = Some(context);
        self
    }

    /// Get a required string argument, returning a `Skill` error if missing or
    /// not a string.
    pub fn get_string(&self, key: &str) -> crate::Result<&str> {
        self.args
            .get(key)
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::Error::Skill(format!("{key} is required")))
    }

    /// Get an optional string argument.
    pub fn get_optional_string(&self, key: &str) -> Option<&str> {
        self.args.get(key).and_then(|v| v.as_str())
    }

    /// Get a required i64 argument.
    pub fn get_i64(&self, key: &str) -> crate::Result<i64> {
        self.args
            .get(key)
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| crate::Error::Skill(format!("{key} is required")))
    }

    /// Get a required bool argument.
    pub fn get_bool(&self, key: &str) -> crate::Result<bool> {
        self.args
            .get(key)
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| crate::Error::Skill(format!("{key} is required")))
    }

    /// Resolve a path string against the user's CWD from context.
    ///
    /// - Relative paths are joined onto `user_cwd` when set.
    /// - Paths starting with `~/` or equal to `~` are treated as relative to
    ///   `user_cwd` (not the server process home), so LLM-generated tilde paths
    ///   land in the user's working directory rather than the server's `$HOME`.
    /// - Absolute paths without a tilde are returned as-is.
    pub fn resolve_path(&self, path: &str) -> std::path::PathBuf {
        let cwd = self.context.as_ref().and_then(|c| c.user_cwd.as_deref());

        let tilde_rest = path
            .strip_prefix("~/")
            .or_else(|| if path == "~" { Some("") } else { None });
        if let (Some(rest), Some(dir)) = (tilde_rest, cwd) {
            return std::path::PathBuf::from(dir).join(rest);
        }

        let p = std::path::Path::new(path);
        if p.is_relative()
            && let Some(dir) = cwd
        {
            return std::path::PathBuf::from(dir).join(p);
        }
        p.to_path_buf()
    }
}

/// Output returned from a skill invocation.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct SkillOutput {
    /// Structured result value produced by the skill.
    pub data: serde_json::Value,
    /// Media attachments produced alongside the text result (e.g. generated
    /// charts). These are forwarded as separate `Payload::Media` messages
    /// to the channel adapter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<MediaPayload>,
}

impl SkillOutput {
    /// Create a new skill output.
    pub fn new(data: serde_json::Value) -> Self {
        Self {
            data,
            attachments: Vec::new(),
        }
    }

    /// Attach media payloads to be forwarded alongside the text response.
    #[must_use]
    pub fn with_attachments(mut self, attachments: Vec<MediaPayload>) -> Self {
        self.attachments = attachments;
        self
    }
}

/// JSON Schema describing a skill's parameters.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
pub struct SkillSchema {
    /// JSON Schema object describing the skill's accepted parameters.
    pub parameters: serde_json::Value,
}

impl SkillSchema {
    /// Create a new skill schema.
    pub fn new(parameters: serde_json::Value) -> Self {
        Self { parameters }
    }
}

/// Kind of principle stored in the experience system.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PrincipleKind {
    /// A positive pattern: something the agent should do.
    Do,
    /// A negative pattern: something the agent should avoid.
    Avoid,
}

/// Controls which soft skills are injected into the agent system prompt.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SoftSkillSelectionMode {
    /// Inject all registered soft skills (default, backward-compatible).
    #[default]
    All,
    /// Inject only soft skills whose name or tags match words in the user's
    /// message. Reduces prompt bloat when many skills are registered.
    Keyword,
}

impl From<&str> for SoftSkillSelectionMode {
    fn from(s: &str) -> Self {
        match s {
            "keyword" => Self::Keyword,
            _ => Self::All,
        }
    }
}
