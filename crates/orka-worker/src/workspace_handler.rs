use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use async_trait::async_trait;
use orka_core::config::AgentConfig;
use orka_core::traits::{EventSink, Guardrail, MemoryStore, SecretManager};
use orka_core::{
    CommandArgs, DomainEvent, DomainEventKind, Envelope, ErrorCategory, MemoryEntry, MessageId,
    OutboundMessage, Payload, Result, Session, SessionId, SkillInput,
};

use orka_experience::ExperienceService;
use orka_llm::client::{
    ChatContent, ChatMessage, CompletionOptions, CompletionResponse, ContentBlock,
    ContentBlockInput, LlmClient, LlmToolStream, StopReason, StreamEvent, ToolCall, ToolDefinition,
    Usage,
};
use orka_llm::context::{
    TokenizerHint, available_history_budget_with_hint, estimate_message_tokens_with_hint,
    truncate_history_with_hint,
};
use orka_skills::SkillRegistry;
use orka_workspace::WorkspaceRegistry;
use tracing::{Instrument, debug, info, info_span, warn};

use crate::commands::CommandRegistry;
use crate::handler::AgentHandler;
use crate::stream::{StreamChunk, StreamChunkKind, StreamRegistry};

/// Truncate a tool result string if it exceeds the configured limit.
fn truncate_tool_result(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        return content.to_string();
    }
    let boundary = content.floor_char_boundary(max_chars);
    let truncated = &content[..boundary];
    format!(
        "{truncated}\n\n[truncated, showing first {boundary} chars of {} total]",
        content.len()
    )
}

/// Derive a category tag and human-readable input summary from the tool name and its JSON input.
fn tool_metadata(name: &str, input: &serde_json::Value) -> (Option<String>, Option<String>) {
    match name {
        "web_search" => {
            let summary = input
                .get("query")
                .and_then(|v| v.as_str())
                .map(|q| format!("query: '{q}'"));
            (Some("search".into()), summary)
        }
        "http_request" => {
            let method = input
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or("GET");
            let summary = input
                .get("url")
                .and_then(|v| v.as_str())
                .map(|u| format!("{method} {u}"));
            (Some("http".into()), summary)
        }
        "sandbox" | "code_exec" | "code_interpreter" => (Some("code".into()), None),
        n if n.starts_with("memory_") || n.starts_with("doc_") => (Some("memory".into()), None),
        n if n.starts_with("schedule_") => (Some("schedule".into()), None),
        _ => (None, None),
    }
}

/// Produce a brief output summary for known tools.
fn summarize_result(name: &str, content: &str, is_error: bool) -> Option<String> {
    if is_error {
        // Truncate long error messages
        let msg = if content.len() > 80 {
            format!("{}…", &content[..80])
        } else {
            content.to_string()
        };
        return Some(msg);
    }
    match name {
        "web_search" => {
            // Try to count results from JSON array
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(content)
                && let Some(arr) = v.as_array()
            {
                return Some(format!("Found {} results", arr.len()));
            }
            Some("Search complete".into())
        }
        "http_request" => {
            let len = content.len();
            if len > 1024 {
                Some(format!("{:.1} KB response", len as f64 / 1024.0))
            } else {
                Some(format!("{len} bytes"))
            }
        }
        _ => None,
    }
}

fn format_current_datetime(timezone: Option<&str>) -> String {
    use chrono::Utc;
    if let Some(tz_name) = timezone {
        if let Ok(tz) = tz_name.parse::<chrono_tz::Tz>() {
            let now = Utc::now().with_timezone(&tz);
            return format!(
                "Current date and time: {} ({})",
                now.format("%A, %B %-d, %Y, %H:%M %Z"),
                tz_name
            );
        }
        tracing::warn!(timezone = %tz_name, "invalid timezone in SOUL.md, falling back to UTC");
    }
    let now = Utc::now();
    format!(
        "Current date and time: {} (UTC)",
        now.format("%A, %B %-d, %Y, %H:%M UTC")
    )
}

/// Configuration parameters for [`WorkspaceHandler`], grouped to reduce constructor arguments.
pub struct WorkspaceHandlerConfig {
    /// LLM and agent tuning parameters.
    pub agent_config: AgentConfig,
    /// Tool names that should never be offered to the LLM.
    pub disabled_tools: HashSet<String>,
    /// Fallback context window size when the model info is unavailable.
    pub default_context_window: u32,
}

/// Sliding-window rate limiter for slash commands, keyed by `(SessionId, command_name)`.
///
/// Stores `(window_start, call_count)` per session+command pair and resets the count
/// after `RATE_WINDOW_SECS` seconds.
struct CommandRateLimiter {
    state: Mutex<HashMap<(SessionId, String), (Instant, u32)>>,
    max_per_window: u32,
    window_secs: u64,
}

impl CommandRateLimiter {
    fn new(max_per_window: u32, window_secs: u64) -> Self {
        Self {
            state: Mutex::new(HashMap::new()),
            max_per_window,
            window_secs,
        }
    }

    /// Returns `true` if the command is allowed, `false` if the limit is exceeded.
    fn check_and_record(&self, session_id: SessionId, command: &str) -> bool {
        let mut guard = self.state.lock().unwrap();
        let key = (session_id, command.to_string());
        let now = Instant::now();
        let entry = guard.entry(key).or_insert((now, 0));
        if now.duration_since(entry.0).as_secs() >= self.window_secs {
            // Window expired — reset.
            *entry = (now, 1);
            true
        } else if entry.1 < self.max_per_window {
            entry.1 += 1;
            true
        } else {
            false
        }
    }
}

/// LLM-powered agent handler with tool-use loops, guardrails, and experience learning.
pub struct WorkspaceHandler {
    workspace_registry: Arc<WorkspaceRegistry>,
    skills: Arc<SkillRegistry>,
    memory: Arc<dyn MemoryStore>,
    secrets: Arc<dyn SecretManager>,
    llm: Option<Arc<dyn LlmClient>>,
    event_sink: Arc<dyn EventSink>,
    agent_config: AgentConfig,
    disabled_tools: HashSet<String>,
    default_context_window: u32,
    guardrail: Option<Arc<dyn Guardrail>>,
    commands: Arc<CommandRegistry>,
    stream_registry: StreamRegistry,
    experience: Option<Arc<ExperienceService>>,
    /// Per-session rate limiter for slash commands (10 per minute by default).
    command_rate_limiter: CommandRateLimiter,
    /// Shared cancellation tokens from the worker pool (used by `/cancel`).
    session_cancel_tokens: Option<crate::SessionCancelTokens>,
}

impl WorkspaceHandler {
    #[allow(clippy::too_many_arguments)]
    /// Create a handler wired to the given registries and stores.
    pub fn new(
        workspace_registry: Arc<WorkspaceRegistry>,
        skills: Arc<SkillRegistry>,
        memory: Arc<dyn MemoryStore>,
        secrets: Arc<dyn SecretManager>,
        llm: Option<Arc<dyn LlmClient>>,
        event_sink: Arc<dyn EventSink>,
        config: WorkspaceHandlerConfig,
        guardrail: Option<Arc<dyn Guardrail>>,
        commands: Arc<CommandRegistry>,
        stream_registry: StreamRegistry,
        experience: Option<Arc<ExperienceService>>,
    ) -> Self {
        Self {
            workspace_registry,
            skills,
            memory,
            secrets,
            llm,
            event_sink,
            agent_config: config.agent_config,
            disabled_tools: config.disabled_tools,
            default_context_window: config.default_context_window,
            guardrail,
            commands,
            stream_registry,
            experience,
            command_rate_limiter: CommandRateLimiter::new(10, 60),
            session_cancel_tokens: None,
        }
    }

    /// Wire in the shared session cancellation token map from the worker pool.
    pub fn with_session_cancel_tokens(mut self, tokens: crate::SessionCancelTokens) -> Self {
        self.session_cancel_tokens = Some(tokens);
        self
    }

    fn make_reply(&self, envelope: &Envelope, text: String) -> OutboundMessage {
        let mut msg = OutboundMessage::text(
            envelope.channel.clone(),
            envelope.session_id,
            text,
            Some(envelope.id),
        );
        msg.metadata = envelope.metadata.clone();
        msg.metadata
            .entry("source_channel".into())
            .or_insert_with(|| serde_json::Value::String(envelope.channel.clone()));
        msg
    }

    /// Build LLM tool definitions from skill registry, excluding disabled tools.
    /// Also appends built-in workspace management tools.
    fn build_tool_definitions(&self) -> Vec<ToolDefinition> {
        let mut defs: Vec<ToolDefinition> = self
            .skills
            .list()
            .iter()
            .filter(|name| !self.disabled_tools.contains(**name))
            .filter_map(|name| self.skills.get(name))
            .map(|skill| {
                ToolDefinition::new(
                    skill.name(),
                    skill.description(),
                    skill.schema().parameters.clone(),
                )
            })
            .collect();

        // Built-in workspace tools
        defs.push(ToolDefinition::new(
            "workspace_info",
            "Get information about the current workspace and list all available workspaces.",
            serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        ));
        defs.push(ToolDefinition::new(
            "workspace_switch",
            "Switch to a different workspace by name. Changes the active persona and tools for this session.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the workspace to switch to"
                    }
                },
                "required": ["name"]
            }),
        ));

        defs
    }

    /// Assemble the system prompt from SOUL.md body, TOOLS.md body, current datetime,
    /// and workspace awareness.
    #[allow(clippy::too_many_arguments)]
    fn build_system_prompt(
        soul_name: &str,
        soul_body: &str,
        tools_body: &str,
        timezone: Option<&str>,
        workspace_name: &str,
        available_workspaces: &[&str],
        principles_section: &str,
        conversation_summary: Option<&str>,
        cwd: Option<&str>,
    ) -> String {
        let mut prompt = if soul_body.is_empty() {
            format!("You are {soul_name}.")
        } else {
            format!("You are {soul_name}.\n\n{soul_body}")
        };

        let now_str = format_current_datetime(timezone);
        prompt.push_str("\n\n");
        prompt.push_str(&now_str);

        if let Some(dir) = cwd {
            prompt.push_str(&format!(
                "\n\nThe user's current working directory is: {dir}\n\
                When the user asks to create, read, or modify files without specifying an absolute \
                path, resolve them relative to this directory. Use this directory as the default \
                working directory for shell commands."
            ));
        }

        if !tools_body.is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(tools_body);
        }

        // Inject learned principles (if any)
        if !principles_section.is_empty() {
            prompt.push_str(principles_section);
        }

        // Workspace awareness
        let ws_list = available_workspaces.join(", ");
        prompt.push_str(&format!(
            "\n\nYou are currently operating in workspace \"{workspace_name}\".\n\
             Available workspaces: {ws_list}.\n\
             You can use the workspace_info tool to get details and workspace_switch to change workspace."
        ));

        // Inject prior conversation summary when the history has been condensed.
        // This is placed after workspace context so the LLM sees it close to the messages.
        if let Some(summary) = conversation_summary.filter(|s| !s.is_empty()) {
            prompt.push_str("\n\n## Prior Conversation Context\n");
            prompt.push_str(summary);
        }

        prompt
    }

    /// Resolve workspace from the registry by name. Falls back to default if not found.
    async fn resolve_from_registry(&self, ws_name: &str) -> (String, String, String) {
        let state_lock = self.workspace_registry.state(ws_name).unwrap_or_else(|| {
            warn!(workspace = %ws_name, "workspace not found in registry, using default");
            self.workspace_registry.default_state()
        });
        let state = state_lock.read().await;
        let soul_name = state
            .soul
            .as_ref()
            .and_then(|doc| doc.frontmatter.name.clone())
            .unwrap_or_else(|| self.agent_config.display_name.clone());
        let soul_body = state
            .soul
            .as_ref()
            .map(|doc| doc.body.clone())
            .unwrap_or_default();
        let tools_body = state.tools_body.clone().unwrap_or_default();
        (soul_name, soul_body, tools_body)
    }

    /// Resolve workspace from inline CLI content (raw SOUL.md/TOOLS.md strings).
    /// Falls back to the default workspace for any missing piece.
    async fn resolve_from_inline(
        &self,
        raw_soul: Option<&str>,
        raw_tools: Option<&str>,
    ) -> (String, String, String) {
        let (name, body) = if let Some(raw) = raw_soul {
            match orka_workspace::parse::parse_document::<orka_workspace::SoulFrontmatter>(raw) {
                Ok(doc) => (
                    doc.frontmatter
                        .name
                        .unwrap_or_else(|| self.agent_config.display_name.clone()),
                    doc.body,
                ),
                Err(e) => {
                    warn!(%e, "failed to parse workspace override SOUL.md, falling back");
                    (self.agent_config.display_name.clone(), raw.to_string())
                }
            }
        } else {
            let state = self.workspace_registry.default_state();
            let state = state.read().await;
            let name = state
                .soul
                .as_ref()
                .and_then(|doc| doc.frontmatter.name.clone())
                .unwrap_or_else(|| self.agent_config.display_name.clone());
            let body = state
                .soul
                .as_ref()
                .map(|doc| doc.body.clone())
                .unwrap_or_default();
            (name, body)
        };

        let tools = if let Some(raw) = raw_tools {
            orka_workspace::strip_frontmatter(raw)
        } else {
            let state = self.workspace_registry.default_state();
            let state = state.read().await;
            state.tools_body.clone().unwrap_or_default()
        };

        (name, body, tools)
    }

    /// Handle a built-in workspace tool call. Returns `Some(result)` if the tool was
    /// handled, or `None` if it should be dispatched to the skill registry.
    async fn handle_builtin_tool(
        &self,
        call: &ToolCall,
        session: &Session,
        current_workspace: &str,
    ) -> Option<(String, bool)> {
        match call.name.as_str() {
            "workspace_info" => {
                let names = self.workspace_registry.list_names();
                let mut workspaces = Vec::new();
                for name in &names {
                    let is_current = *name == current_workspace;
                    let soul_name = if let Some(state_lock) = self.workspace_registry.state(name) {
                        let state = state_lock.read().await;
                        state
                            .soul
                            .as_ref()
                            .and_then(|doc| doc.frontmatter.name.clone())
                    } else {
                        None
                    };
                    workspaces.push(serde_json::json!({
                        "name": name,
                        "soul_name": soul_name,
                        "active": is_current,
                    }));
                }
                let result = serde_json::json!({
                    "current_workspace": current_workspace,
                    "workspaces": workspaces,
                });
                Some((result.to_string(), false))
            }
            "workspace_switch" => {
                let target = call
                    .input
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if target.is_empty() {
                    return Some(("Error: missing required parameter 'name'".into(), true));
                }
                if self.workspace_registry.get(target).is_none() {
                    let available = self.workspace_registry.list_names().join(", ");
                    return Some((
                        format!("Error: workspace '{target}' not found. Available: {available}"),
                        true,
                    ));
                }
                let override_key = format!("workspace_override:{}", session.id);
                let override_val = serde_json::json!({ "workspace_name": target });
                let entry = MemoryEntry::new(override_key.clone(), override_val);
                if let Err(e) = self.memory.store(&override_key, entry, None).await {
                    return Some((format!("Error storing workspace override: {e}"), true));
                }
                info!(from = %current_workspace, to = %target, session = %session.id, "workspace switched");
                Some((
                    format!(
                        "Switched to workspace '{target}'. The new persona and tools will take effect on the next message."
                    ),
                    false,
                ))
            }
            _ => None,
        }
    }

    /// Execute tool calls in parallel, emitting streaming events and domain events.
    /// Returns content blocks with tool results in the original call order.
    /// Built-in workspace tools are intercepted before dispatching to the skill registry.
    async fn execute_tool_calls(
        &self,
        tool_calls: &[ToolCall],
        envelope: &Envelope,
        session: &Session,
        current_workspace: &str,
    ) -> (Vec<ContentBlockInput>, Vec<Option<ErrorCategory>>) {
        let mut join_set = tokio::task::JoinSet::new();
        // Track which calls are handled as built-in (index → result)
        let mut builtin_results: HashMap<usize, (String, bool)> = HashMap::new();

        for (idx, call) in tool_calls.iter().enumerate() {
            // Check for built-in workspace tools first
            if let Some(result) = self
                .handle_builtin_tool(call, session, current_workspace)
                .await
            {
                info!(tool = %call.name, id = %call.id, "handled built-in workspace tool");
                builtin_results.insert(idx, result);
                continue;
            }

            // Tool input guardrail: check serialized args before executing the skill.
            // Blocked calls are returned to the LLM as an error result (no execution).
            // Modified args replace the original input for the spawned task.
            let mut call_input_override: Option<serde_json::Value> = None;
            if let Some(ref guardrail) = self.guardrail {
                let input_json = call.input.to_string();
                match guardrail.check_input(&input_json, session).await {
                    Ok(orka_core::traits::GuardrailDecision::Block(reason)) => {
                        warn!(skill = %call.name, %reason, "tool input blocked by guardrail");
                        builtin_results.insert(
                            idx,
                            (format!("Tool input blocked by guardrail: {reason}"), true),
                        );
                        continue;
                    }
                    Ok(orka_core::traits::GuardrailDecision::Modify(modified)) => {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&modified) {
                            call_input_override = Some(v);
                        }
                    }
                    _ => {}
                }
            }

            info!(skill = %call.name, id = %call.id, "invoking skill via tool call");

            self.event_sink
                .emit(DomainEvent::new(DomainEventKind::SkillInvoked {
                    skill_name: call.name.clone(),
                    message_id: envelope.id,
                    input_args: match &call.input {
                        serde_json::Value::Object(map) => {
                            map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
                        }
                        _ => HashMap::new(),
                    },
                    caller_id: None,
                }))
                .await;

            let (category, input_summary) = tool_metadata(&call.name, &call.input);
            self.stream_registry.send(StreamChunk::new(
                envelope.session_id,
                envelope.channel.clone(),
                Some(envelope.id),
                StreamChunkKind::ToolExecStart {
                    name: call.name.clone(),
                    id: call.id.clone(),
                    input_summary,
                    category,
                },
            ));

            let call_id = call.id.clone();
            let call_name = call.name.clone();
            let call_name_for_summary = call.name.clone();
            let call_input = call_input_override.unwrap_or_else(|| call.input.clone());
            let skills = self.skills.clone();
            let event_sink = self.event_sink.clone();
            let message_id = envelope.id;
            let secrets = self.secrets.clone();
            let skill_timeout =
                std::time::Duration::from_secs(self.agent_config.skill_timeout_secs);
            let max_result_chars = self.agent_config.max_tool_result_chars;
            let user_cwd = envelope
                .metadata
                .get("workspace:cwd")
                .and_then(|v| v.as_str())
                .map(String::from);

            join_set.spawn(async move {
                let args: HashMap<String, serde_json::Value> = match call_input {
                    serde_json::Value::Object(map) => map.into_iter().collect(),
                    _ => HashMap::new(),
                };

                let start = std::time::Instant::now();
                let skill_input = SkillInput::new(args).with_context(
                    orka_core::SkillContext::new(secrets, Some(event_sink.clone()))
                        .with_user_cwd(user_cwd),
                );
                let result = match tokio::time::timeout(
                    skill_timeout,
                    skills.invoke(&call_name, skill_input),
                )
                .await
                {
                    Ok(r) => r,
                    Err(_) => Err(orka_core::Error::Skill(format!(
                        "skill '{call_name}' timed out after {}s",
                        skill_timeout.as_secs()
                    ))),
                };
                let duration_ms = start.elapsed().as_millis() as u64;

                let error_category = match &result {
                    Err(e) => Some(e.category()),
                    Ok(_) => None,
                };
                let (content, is_error) = match &result {
                    Ok(output) => {
                        let raw = output.data.to_string();
                        (truncate_tool_result(&raw, max_result_chars), false)
                    }
                    Err(e) => (format!("Error: {e}"), true),
                };

                let output_preview = match &result {
                    Ok(output) => {
                        let s = output.data.to_string();
                        Some(s.chars().take(1024).collect::<String>())
                    }
                    Err(_) => None,
                };
                let error_message = match &result {
                    Err(e) => Some(e.to_string()),
                    Ok(_) => None,
                };

                event_sink
                    .emit(DomainEvent::new(DomainEventKind::SkillCompleted {
                        skill_name: call_name,
                        message_id,
                        duration_ms,
                        success: !is_error,
                        error_category,
                        output_preview,
                        error_message,
                    }))
                    .await;

                let error_msg = if is_error {
                    Some(content.clone())
                } else {
                    None
                };
                let result_summary = summarize_result(&call_name_for_summary, &content, is_error);
                (
                    call_id,
                    content,
                    is_error,
                    duration_ms,
                    error_msg,
                    result_summary,
                    error_category,
                )
            });
        }

        // Collect results and emit ToolExecEnd chunks
        let mut results_map: HashMap<String, (String, bool, Option<ErrorCategory>)> =
            HashMap::new();
        while let Some(res) = join_set.join_next().await {
            if let Ok((
                call_id,
                content,
                is_error,
                duration_ms,
                error_msg,
                result_summary,
                error_category,
            )) = res
            {
                self.stream_registry.send(StreamChunk::new(
                    envelope.session_id,
                    envelope.channel.clone(),
                    Some(envelope.id),
                    StreamChunkKind::ToolExecEnd {
                        id: call_id.clone(),
                        success: !is_error,
                        duration_ms,
                        error: error_msg,
                        result_summary,
                    },
                ));
                results_map.insert(call_id, (content, is_error, error_category));
            }
        }

        // Build result blocks in original order, merging built-in and skill results
        let mut blocks = Vec::with_capacity(tool_calls.len());
        let mut categories = Vec::with_capacity(tool_calls.len());
        for (idx, call) in tool_calls.iter().enumerate() {
            let (content, is_error, category) =
                if let Some((content, is_error)) = builtin_results.remove(&idx) {
                    (content, is_error, None)
                } else {
                    results_map
                        .remove(&call.id)
                        .unwrap_or_else(|| ("Error: task failed".to_string(), true, None))
                };
            blocks.push(ContentBlockInput::ToolResult {
                tool_use_id: call.id.clone(),
                content,
                is_error,
            });
            categories.push(category);
        }
        (blocks, categories)
    }

    /// Compact oversized tool results — delegates to the shared [`crate::history`] module.
    fn compact_tool_results(messages: Vec<ChatMessage>, max_chars: usize) -> Vec<ChatMessage> {
        crate::history::compact_tool_results(messages, max_chars)
    }

    /// Persist conversation history and token usage to the memory store.
    ///
    /// When the history exceeds `max_entries` the oldest messages are summarised
    /// using incremental rolling summarisation.  The resulting summary text is stored
    /// separately under `summary_key` and injected into the system prompt on the next
    /// turn via [`Self::build_system_prompt`].
    #[allow(clippy::too_many_arguments)]
    async fn save_conversation_history(
        &self,
        messages: Vec<ChatMessage>,
        max_entries: usize,
        summarization_model: Option<&str>,
        existing_summary: Option<&str>,
        memory_key: &str,
        summary_key: &str,
        session_tokens: u64,
        token_key: &str,
    ) {
        // Compact verbose tool results before persisting to keep storage lean.
        const MAX_TOOL_RESULT_CHARS: usize = 2000;
        let messages = Self::compact_tool_results(messages, MAX_TOOL_RESULT_CHARS);

        let history_to_save = if messages.len() > max_entries {
            let split_point = messages.len() - max_entries;
            let old_messages = &messages[..split_point];

            let summary_text = if let Some(llm) = &self.llm {
                Self::summarize_messages(llm, old_messages, summarization_model, existing_summary)
                    .await
            } else {
                // No LLM configured: fall back to bullet-point extraction.
                Self::fallback_summary(old_messages)
            };

            // Persist the summary separately so it can be injected into the system
            // prompt on the next request without polluting the message list.
            let summary_entry = MemoryEntry::new(summary_key, serde_json::json!(summary_text))
                .with_tags(vec!["conversation_summary".to_string()]);
            if let Err(e) = self.memory.store(summary_key, summary_entry, None).await {
                warn!(%e, key = %summary_key, "failed to persist conversation summary");
            }

            messages[split_point..].to_vec()
        } else {
            messages
        };

        let history_value = match serde_json::to_value(&history_to_save) {
            Ok(v) => v,
            Err(e) => {
                warn!(%e, key = %memory_key, "failed to serialize conversation history");
                return;
            }
        };
        let entry =
            MemoryEntry::new(memory_key, history_value).with_tags(vec!["conversation".to_string()]);
        if let Err(e) = self.memory.store(memory_key, entry, None).await {
            warn!(%e, key = %memory_key, "failed to persist conversation history");
        }

        let token_entry = MemoryEntry::new(token_key, serde_json::json!(session_tokens))
            .with_tags(vec!["token_usage".to_string()]);
        if let Err(e) = self.memory.store(token_key, token_entry, None).await {
            warn!(%e, key = %token_key, "failed to persist token usage");
        }
    }

    /// Produce a plain-text transcript excerpt from a slice of messages.
    fn build_transcript(messages: &[ChatMessage]) -> String {
        let mut transcript = String::new();
        for msg in messages {
            let text = match &msg.content {
                ChatContent::Text(t) => t.clone(),
                ChatContent::Blocks(blocks) => {
                    let mut parts = Vec::new();
                    for b in blocks {
                        match b {
                            ContentBlockInput::Text { text } => parts.push(text.clone()),
                            ContentBlockInput::ToolUse { name, .. } => {
                                parts.push(format!("[called {name}]"))
                            }
                            ContentBlockInput::ToolResult { content, .. } => {
                                // Keep tool results brief in the transcript.
                                let excerpt = if content.len() > 200 {
                                    format!("{}…", &content[..200])
                                } else {
                                    content.clone()
                                };
                                parts.push(format!("[result: {excerpt}]"))
                            }
                            _ => continue,
                        }
                    }
                    if parts.is_empty() {
                        "[tool interaction]".to_string()
                    } else {
                        parts.join(" ")
                    }
                }
                _ => "[unsupported content]".to_string(),
            };
            transcript.push_str(&format!("{}: {}\n", msg.role, text));
        }
        transcript
    }

    /// Build a minimal summary from user-text messages when LLM summarisation is unavailable.
    fn fallback_summary(messages: &[ChatMessage]) -> String {
        use orka_llm::client::Role;
        let bullets: Vec<String> = messages
            .iter()
            .filter(|m| m.role == Role::User)
            .filter_map(|m| match &m.content {
                ChatContent::Text(t) if !t.is_empty() => {
                    Some(format!("- {}", t.chars().take(120).collect::<String>()))
                }
                ChatContent::Blocks(blocks) => {
                    let text: String = blocks
                        .iter()
                        .filter_map(|b| {
                            if let ContentBlockInput::Text { text } = b {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    if text.is_empty() {
                        None
                    } else {
                        Some(format!("- {}", text.chars().take(120).collect::<String>()))
                    }
                }
                _ => None,
            })
            .collect();

        if bullets.is_empty() {
            format!("[{} messages truncated]", messages.len())
        } else {
            format!(
                "Previous conversation (auto-summarized):\n{}",
                bullets.join("\n")
            )
        }
    }

    /// Summarise a slice of messages, optionally updating an existing rolling summary.
    ///
    /// When `existing_summary` is provided the LLM is asked to update it with the new
    /// turns, preserving user goals and unresolved tasks (incremental rolling pattern).
    async fn summarize_messages(
        llm: &Arc<dyn LlmClient>,
        messages: &[ChatMessage],
        model: Option<&str>,
        existing_summary: Option<&str>,
    ) -> String {
        use orka_llm::client::ChatMessage;

        let transcript = Self::build_transcript(messages);

        let prompt_text = if let Some(old) = existing_summary {
            format!(
                "Update this existing summary with the new conversation turns. \
                 Preserve user goals, constraints, and unresolved tasks.\n\n\
                 Existing summary:\n{old}\n\nNew turns:\n{transcript}"
            )
        } else {
            format!(
                "Summarize the following conversation concisely, preserving \
                 key facts, decisions, and context:\n\n{transcript}"
            )
        };

        let summary_prompt = vec![ChatMessage::user(prompt_text)];

        let mut options = CompletionOptions::default();
        options.model = model.map(|s| s.to_string());
        options.max_tokens = Some(1024);

        match llm
            .complete_with_options(
                summary_prompt,
                "You are a conversation summarizer. Be concise.",
                options,
            )
            .await
        {
            Ok(summary) => summary,
            Err(e) => {
                tracing::warn!(%e, "failed to summarize conversation, using fallback");
                Self::fallback_summary(messages)
            }
        }
    }

    /// Consume a streaming LLM response, emitting `StreamChunk`s to the registry
    /// and reconstructing a `CompletionResponse` from the events.
    async fn consume_stream(
        mut stream: LlmToolStream,
        stream_registry: &StreamRegistry,
        session_id: &SessionId,
        channel: &str,
        reply_to: Option<&MessageId>,
    ) -> Result<CompletionResponse> {
        use futures_util::StreamExt;

        let mut text = String::new();
        let mut thinking = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut current_tool_id: Option<String> = None;
        let mut current_tool_name: Option<String> = None;
        let mut current_tool_input = String::new();
        let mut usage = Usage::default();
        let mut stop_reason = None;

        while let Some(event) = stream.next().await {
            let event = event?;
            match event {
                StreamEvent::ThinkingDelta(delta) => {
                    stream_registry.send(StreamChunk::new(
                        *session_id,
                        channel.to_string(),
                        reply_to.copied(),
                        StreamChunkKind::ThinkingDelta(delta.clone()),
                    ));
                    thinking.push_str(&delta);
                }
                StreamEvent::TextDelta(delta) => {
                    stream_registry.send(StreamChunk::new(
                        *session_id,
                        channel.to_string(),
                        reply_to.copied(),
                        StreamChunkKind::Delta(delta.clone()),
                    ));
                    text.push_str(&delta);
                }
                StreamEvent::ToolUseStart { id, name } => {
                    stream_registry.send(StreamChunk::new(
                        *session_id,
                        channel.to_string(),
                        reply_to.copied(),
                        StreamChunkKind::ToolStart {
                            name: name.clone(),
                            id: id.clone(),
                        },
                    ));
                    current_tool_id = Some(id);
                    current_tool_name = Some(name);
                    current_tool_input.clear();
                }
                StreamEvent::ToolUseInputDelta(delta) => {
                    current_tool_input.push_str(&delta);
                }
                StreamEvent::ToolUseEnd { id, input } => {
                    let name = current_tool_name.take().unwrap_or_default();
                    let final_input = if input != serde_json::Value::Null {
                        input
                    } else {
                        serde_json::from_str(&current_tool_input).unwrap_or_else(|e| {
                            warn!(%e, tool = %name, "malformed tool input JSON, using empty object");
                            serde_json::Value::Object(Default::default())
                        })
                    };
                    stream_registry.send(StreamChunk::new(
                        *session_id,
                        channel.to_string(),
                        reply_to.copied(),
                        StreamChunkKind::ToolEnd {
                            id: id.clone(),
                            success: true,
                        },
                    ));
                    tool_calls.push(ToolCall::new(id, name, final_input));
                    current_tool_id = None;
                    current_tool_input.clear();
                }
                StreamEvent::Usage(u) => usage = u,
                StreamEvent::Stop(reason) => stop_reason = Some(reason),
                other => {
                    tracing::debug!(?other, "unhandled stream event");
                }
            }
        }

        // If we were mid-tool when the stream ended, treat it as incomplete
        if let Some(id) = current_tool_id {
            stream_registry.send(StreamChunk::new(
                *session_id,
                channel.to_string(),
                reply_to.copied(),
                StreamChunkKind::ToolEnd { id, success: false },
            ));
        }

        let mut blocks = Vec::new();
        if !thinking.is_empty() {
            blocks.push(ContentBlock::Thinking(thinking));
        }
        if !text.is_empty() {
            blocks.push(ContentBlock::Text(text));
        }
        for call in tool_calls {
            blocks.push(ContentBlock::ToolUse(call));
        }

        Ok(CompletionResponse::new(blocks, usage, stop_reason))
    }
}

#[async_trait]
impl AgentHandler for WorkspaceHandler {
    async fn handle(&self, envelope: &Envelope, session: &Session) -> Result<Vec<OutboundMessage>> {
        // Dispatch structured commands directly without round-tripping through text.
        if let Payload::Command(cmd) = &envelope.payload {
            if !self
                .command_rate_limiter
                .check_and_record(envelope.session_id, &cmd.name)
            {
                return Ok(vec![
                    self.make_reply(
                        envelope,
                        "Rate limit exceeded. Please wait a moment before sending another command."
                            .into(),
                    ),
                ]);
            }
            let args = CommandArgs::from(cmd.clone());
            if let Some(handler) = self.commands.get(&cmd.name) {
                return handler.execute(&args, envelope, session).await;
            }
            let help = self.commands.help_text();
            return Ok(vec![self.make_reply(
                envelope,
                format!("Unknown command: /{}\n\n{help}", cmd.name),
            )]);
        }

        let text = match &envelope.payload {
            Payload::Text(t) => t.clone(),
            _ => {
                return Ok(vec![self.make_reply(
                    envelope,
                    "Sorry, I can only process text messages.".into(),
                )]);
            }
        };

        // Dispatch ALL slash commands before the guardrail.  Commands are trusted internal
        // handlers — there is no reason to run a guardrail check on them.
        if let Some(parsed) = orka_core::parse_slash_command(&text) {
            if !self
                .command_rate_limiter
                .check_and_record(envelope.session_id, &parsed.name)
            {
                return Ok(vec![
                    self.make_reply(
                        envelope,
                        "Rate limit exceeded. Please wait a moment before sending another command."
                            .into(),
                    ),
                ]);
            }
            let args = CommandArgs::from(parsed.clone());
            if let Some(handler) = self.commands.get(&parsed.name) {
                return handler.execute(&args, envelope, session).await;
            }
            // Unknown slash command — show help
            let help = self.commands.help_text();
            return Ok(vec![self.make_reply(
                envelope,
                format!("Unknown command: /{}\n\n{help}", parsed.name),
            )]);
        }

        // 3-tier workspace resolution:
        //   1. Per-session override from MemoryStore (CLI inline content or named workspace)
        //   2. Adapter-level workspace binding (workspace:name in envelope metadata)
        //   3. Default workspace from registry
        let override_key = format!("workspace_override:{}", session.id);

        // Persist CLI metadata overrides into MemoryStore for session stickiness
        let has_ws_meta = envelope.metadata.contains_key("workspace:soul")
            || envelope.metadata.contains_key("workspace:tools")
            || envelope.metadata.contains_key("workspace:cwd");

        if has_ws_meta {
            let override_val = serde_json::json!({
                "soul": envelope.metadata.get("workspace:soul").and_then(|v| v.as_str()),
                "tools": envelope.metadata.get("workspace:tools").and_then(|v| v.as_str()),
                "cwd": envelope.metadata.get("workspace:cwd").and_then(|v| v.as_str()),
            });
            if let Err(e) = self
                .memory
                .store(
                    &override_key,
                    MemoryEntry::new(override_key.clone(), override_val),
                    None,
                )
                .await
            {
                warn!(%e, "failed to store workspace override");
            }
        }

        let ws_override = self.memory.recall(&override_key).await.ok().flatten();

        let (soul_name, soul_body, tools_body) = if let Some(ref entry) = ws_override {
            // Case 1a: named workspace override (e.g. stored by a prior API call)
            if let Some(ws_name) = entry.value.get("workspace_name").and_then(|v| v.as_str()) {
                self.resolve_from_registry(ws_name).await
            }
            // Case 1b: inline content override (CLI sends raw SOUL.md/TOOLS.md content)
            else {
                let raw_soul = entry.value.get("soul").and_then(|v| v.as_str());
                let raw_tools = entry.value.get("tools").and_then(|v| v.as_str());
                self.resolve_from_inline(raw_soul, raw_tools).await
            }
        } else if let Some(ws_name) = envelope
            .metadata
            .get("workspace:name")
            .and_then(|v| v.as_str())
        {
            // Case 2: adapter-level workspace binding
            self.resolve_from_registry(ws_name).await
        } else {
            // Case 3: default workspace
            self.resolve_from_registry(self.workspace_registry.default_name())
                .await
        };

        // Runtime params from agent config
        let agent = &self.agent_config;
        let soul_model = agent.model.clone();
        let soul_max_tokens = agent.max_tokens;
        let max_tokens_per_session = agent.max_tokens_per_session;
        let summarization_model = agent.summarization_model.clone();
        let max_entries = agent.max_history_entries;
        let context_window = agent
            .context_window_tokens
            .unwrap_or(self.default_context_window);
        let soul_timezone = agent.timezone.clone();
        let max_iterations = agent.max_iterations;

        // Apply input guardrail
        let text = if let Some(ref guardrail) = self.guardrail {
            match guardrail.check_input(&text, session).await? {
                orka_core::traits::GuardrailDecision::Allow => text,
                orka_core::traits::GuardrailDecision::Block(reason) => {
                    return Ok(vec![self.make_reply(
                        envelope,
                        format!("I can't process that request: {reason}"),
                    )]);
                }
                orka_core::traits::GuardrailDecision::Modify(filtered) => filtered,
                _ => text,
            }
        } else {
            text
        };

        // If LLM is configured, run the agent loop
        if let Some(ref llm) = self.llm {
            let tools = self.build_tool_definitions();

            // Load conversation history, summary, and token usage in parallel
            let memory_key = format!("conversation:{}", session.id);
            let summary_key = format!("conversation_summary:{}", session.id);
            let token_key = format!("tokens:{}", session.id);
            let (history_result, summary_result, tokens_result) = tokio::join!(
                self.memory.recall(&memory_key),
                self.memory.recall(&summary_key),
                self.memory.recall(&token_key),
            );

            let history: Vec<ChatMessage> = match history_result {
                Ok(Some(entry)) => {
                    serde_json::from_value(entry.value).unwrap_or_else(|e| {
                        warn!(%e, key = %memory_key, "failed to deserialize conversation history, starting fresh");
                        Vec::new()
                    })
                }
                Ok(None) => Vec::new(),
                Err(e) => {
                    warn!(%e, key = %memory_key, "failed to recall conversation history");
                    Vec::new()
                }
            };

            let conversation_summary: Option<String> = match summary_result {
                Ok(Some(entry)) => entry.value.as_str().map(|s| s.to_string()),
                Ok(None) => None,
                Err(e) => {
                    warn!(%e, key = %summary_key, "failed to recall conversation summary");
                    None
                }
            };

            let mut session_tokens: u64 = match tokens_result {
                Ok(Some(entry)) => match entry.value.as_u64() {
                    Some(v) => v,
                    None => {
                        warn!(key = %token_key, value = ?entry.value, "corrupted token count, resetting to 0");
                        0
                    }
                },
                Ok(None) => 0,
                Err(e) => {
                    warn!(%e, key = %token_key, "failed to recall token usage");
                    0
                }
            };

            let mut messages = history;
            messages.push(ChatMessage::user(text.clone()));

            let available_ws = self.workspace_registry.list_names();
            let available_ws_refs: Vec<&str> = available_ws.to_vec();

            // Determine the resolved workspace name for awareness
            let resolved_workspace = if let Some(ref entry) = ws_override {
                if let Some(ws_name) = entry.value.get("workspace_name").and_then(|v| v.as_str()) {
                    ws_name.to_string()
                } else {
                    self.workspace_registry.default_name().to_string()
                }
            } else if let Some(ws_name) = envelope
                .metadata
                .get("workspace:name")
                .and_then(|v| v.as_str())
            {
                ws_name.to_string()
            } else {
                self.workspace_registry.default_name().to_string()
            };

            // Retrieve learned principles for prompt injection
            let principles_section = if let Some(ref exp) = self.experience {
                match exp.retrieve_principles(&text, &resolved_workspace).await {
                    Ok(principles) if !principles.is_empty() => {
                        self.event_sink
                            .emit(DomainEvent::new(DomainEventKind::PrinciplesInjected {
                                session_id: envelope.session_id,
                                count: principles.len(),
                            }))
                            .await;
                        self.stream_registry.send(StreamChunk::new(
                            envelope.session_id,
                            envelope.channel.clone(),
                            Some(envelope.id),
                            StreamChunkKind::PrinciplesUsed {
                                count: principles.len() as u32,
                            },
                        ));
                        ExperienceService::format_principles_section(&principles)
                    }
                    Ok(_) => String::new(),
                    Err(e) => {
                        warn!(%e, "failed to retrieve principles, continuing without");
                        String::new()
                    }
                }
            } else {
                String::new()
            };

            // Initialize trajectory collector for experience learning
            let mut trajectory_collector = self.experience.as_ref().and_then(|exp| {
                if exp.is_enabled() {
                    Some(exp.collector(
                        envelope.session_id.to_string(),
                        resolved_workspace.clone(),
                        text.clone(),
                    ))
                } else {
                    None
                }
            });

            let user_cwd = envelope
                .metadata
                .get("workspace:cwd")
                .and_then(|v| v.as_str());
            let mut system_prompt = Self::build_system_prompt(
                &soul_name,
                &soul_body,
                &tools_body,
                soul_timezone.as_deref(),
                &resolved_workspace,
                &available_ws_refs,
                &principles_section,
                conversation_summary.as_deref(),
                user_cwd,
            );

            // Inject recent local shell commands so the AI has context about what the user ran.
            if let Some(shell_ctx) = envelope
                .metadata
                .get("shell:recent_commands")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                system_prompt.push_str("\n\n## Recent local shell commands\n");
                system_prompt.push_str(shell_ctx);
            }

            let mut options = CompletionOptions::default();
            options.model = soul_model.clone();
            options.max_tokens = soul_max_tokens;

            // Agent loop: call LLM, execute tool calls, feed results back
            let mut final_text = String::new();
            let llm_model = soul_model.as_deref().unwrap_or("default").to_string();
            let max_tool_retries = self.agent_config.max_tool_retries;
            // Track per-tool-name consecutive error counts for self-correction
            let mut tool_error_counts: HashMap<String, u32> = HashMap::new();
            for iteration in 0..max_iterations {
                // Check for cancellation between iterations.
                if let Some(ref tokens) = self.session_cancel_tokens {
                    let cancelled = tokens
                        .lock()
                        .unwrap()
                        .get(&envelope.session_id)
                        .map(|t| t.is_cancelled())
                        .unwrap_or(false);
                    if cancelled {
                        info!(session_id = %envelope.session_id, "operation cancelled by user");
                        final_text = "Operation cancelled.".to_string();
                        break;
                    }
                }

                let iteration_start = std::time::Instant::now();

                // Pre-flight truncation: ensure messages fit context window
                let output_budget = soul_max_tokens.unwrap_or(4096);
                let hint = TokenizerHint::from_model(soul_model.as_deref());
                let budget = available_history_budget_with_hint(
                    context_window,
                    output_budget,
                    &system_prompt,
                    &tools,
                    hint,
                );
                let (truncated, dropped) = truncate_history_with_hint(messages, budget, hint, true);
                messages = truncated;
                if dropped > 0 {
                    warn!(
                        dropped,
                        remaining = messages.len(),
                        budget,
                        "truncated history to fit context window"
                    );
                    let history_tokens: u32 = messages
                        .iter()
                        .map(|m| estimate_message_tokens_with_hint(m, hint))
                        .sum();
                    self.stream_registry.send(StreamChunk::new(
                        envelope.session_id,
                        envelope.channel.clone(),
                        Some(envelope.id),
                        StreamChunkKind::ContextInfo {
                            history_tokens,
                            context_window,
                            messages_truncated: dropped as u32,
                            summary_generated: false,
                        },
                    ));
                }

                let llm_start = std::time::Instant::now();
                let llm_span = info_span!(
                    "llm.call",
                    iteration,
                    model = %llm_model,
                    message_id = %envelope.id,
                );
                let stream = match llm
                    .complete_stream_with_tools(&messages, &system_prompt, &tools, options.clone())
                    .instrument(llm_span.clone())
                    .await
                {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(%e, "LLM stream init failed");
                        final_text = format!(
                            "Sorry, the LLM request failed: {e}\n\nPlease check the server logs for details."
                        );
                        break;
                    }
                };
                let completion = match Self::consume_stream(
                    stream,
                    &self.stream_registry,
                    &envelope.session_id,
                    &envelope.channel,
                    Some(&envelope.id),
                )
                .instrument(llm_span)
                .await
                {
                    Ok(resp) => resp,
                    Err(e) => {
                        warn!(%e, "LLM stream failed");
                        final_text = format!(
                            "Sorry, the LLM request failed: {e}\n\nPlease check the server logs for details."
                        );
                        break;
                    }
                };
                let llm_duration_ms = llm_start.elapsed().as_millis() as u64;

                debug!(
                    input_tokens = completion.usage.input_tokens,
                    output_tokens = completion.usage.output_tokens,
                    ?completion.stop_reason,
                    "LLM response received"
                );

                self.event_sink
                    .emit(DomainEvent::new(DomainEventKind::LlmCompleted {
                        message_id: envelope.id,
                        model: llm_model.clone(),
                        provider: if llm_model.contains("claude") {
                            "anthropic".into()
                        } else if llm_model.contains("gpt") || llm_model.contains("o1") {
                            "openai".into()
                        } else {
                            "unknown".into()
                        },
                        input_tokens: completion.usage.input_tokens,
                        output_tokens: completion.usage.output_tokens,
                        reasoning_tokens: completion.usage.reasoning_tokens,
                        duration_ms: llm_duration_ms,
                        estimated_cost_usd: None,
                    }))
                    .await;

                self.stream_registry.send(StreamChunk::new(
                    envelope.session_id,
                    envelope.channel.clone(),
                    Some(envelope.id),
                    StreamChunkKind::Usage {
                        input_tokens: completion.usage.input_tokens,
                        output_tokens: completion.usage.output_tokens,
                        cache_read_tokens: (completion.usage.cache_read_input_tokens > 0)
                            .then_some(completion.usage.cache_read_input_tokens),
                        cache_creation_tokens: (completion.usage.cache_creation_input_tokens > 0)
                            .then_some(completion.usage.cache_creation_input_tokens),
                        reasoning_tokens: (completion.usage.reasoning_tokens > 0)
                            .then_some(completion.usage.reasoning_tokens),
                        model: llm_model.clone(),
                        cost_usd: None,
                    },
                ));

                // Accumulate token usage
                let iteration_tokens =
                    (completion.usage.input_tokens + completion.usage.output_tokens) as u64;
                session_tokens += iteration_tokens;

                // Record iteration in trajectory collector
                if let Some(ref mut tc) = trajectory_collector {
                    tc.record_iteration(iteration_tokens);
                }

                // Check token budget
                if let Some(budget) = max_tokens_per_session
                    && session_tokens > budget
                {
                    warn!(session_tokens, budget, "token budget exceeded for session");
                    final_text = format!(
                        "I've reached the token budget for this session ({budget} tokens). Please start a new conversation."
                    );
                    break;
                }

                if completion.stop_reason == Some(StopReason::MaxTokens) {
                    warn!("LLM response truncated (max_tokens reached)");
                }

                // Collect thinking, text and tool calls from response
                let mut thinking_text = String::new();
                let mut response_text = String::new();
                let mut tool_calls = Vec::new();
                for block in &completion.blocks {
                    match block {
                        ContentBlock::Thinking(t) => thinking_text.push_str(t),
                        ContentBlock::Text(t) => response_text.push_str(t),
                        ContentBlock::ToolUse(call) => tool_calls.push(call.clone()),
                        other => {
                            tracing::debug!(?other, "unhandled content block type");
                        }
                    }
                }

                // Emit AgentReasoning only when extended thinking produced content
                if !thinking_text.is_empty() {
                    self.event_sink
                        .emit(DomainEvent::new(DomainEventKind::AgentReasoning {
                            message_id: envelope.id,
                            iteration,
                            reasoning_text: thinking_text,
                        }))
                        .await;
                }

                if tool_calls.is_empty() {
                    // No tool calls — final response
                    final_text = response_text;
                    messages.push(ChatMessage::assistant(final_text.clone()));
                    // Emit iteration event before breaking
                    self.event_sink
                        .emit(DomainEvent::new(DomainEventKind::AgentIteration {
                            message_id: envelope.id,
                            iteration,
                            tool_count: 0,
                            tokens_used: iteration_tokens,
                            elapsed_ms: iteration_start.elapsed().as_millis() as u64,
                        }))
                        .await;
                    break;
                }

                // LLM requested tool calls
                debug!(
                    iteration,
                    tool_count = tool_calls.len(),
                    "agent loop: executing tool calls"
                );

                // Record assistant message with text + tool_use blocks
                {
                    let mut blocks = Vec::new();
                    if !response_text.is_empty() {
                        blocks.push(ContentBlockInput::Text {
                            text: response_text,
                        });
                    }
                    for call in &tool_calls {
                        blocks.push(ContentBlockInput::ToolUse {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            input: call.input.clone(),
                        });
                    }
                    messages.push(ChatMessage::new(
                        orka_llm::client::Role::Assistant,
                        ChatContent::Blocks(blocks),
                    ));
                }

                // Execute tool calls and collect results
                let tool_span = info_span!(
                    "tools.execute",
                    iteration,
                    tool_count = tool_calls.len(),
                    message_id = %envelope.id,
                );
                let (result_blocks, block_error_cats) = self
                    .execute_tool_calls(&tool_calls, envelope, session, &resolved_workspace)
                    .instrument(tool_span)
                    .await;

                // R1.3: Track per-tool error counts and inject self-correction hints
                let mut corrected_blocks = result_blocks;
                for ((block, error_cat), call) in corrected_blocks
                    .iter()
                    .zip(block_error_cats.iter())
                    .zip(tool_calls.iter())
                {
                    if let ContentBlockInput::ToolResult {
                        is_error, content, ..
                    } = block
                    {
                        let error_msg = if *is_error {
                            Some(content.clone())
                        } else {
                            None
                        };
                        // Record skill in trajectory collector
                        if let Some(ref mut tc) = trajectory_collector {
                            tc.record_skill(
                                call.name.clone(),
                                0,
                                !*is_error,
                                *error_cat,
                                error_msg,
                            );
                        }
                        if *is_error {
                            let count = tool_error_counts.entry(call.name.clone()).or_insert(0);
                            *count += 1;
                            if let Some(ref mut tc) = trajectory_collector {
                                tc.record_error(format!("skill '{}' failed", call.name));
                            }
                        } else {
                            tool_error_counts.remove(&call.name);
                        }
                    }
                }
                // Inject self-correction hint if any tool has failed too many times
                let mut hint_text: Option<String> = None;
                for (tool_name, count) in &tool_error_counts {
                    if *count >= max_tool_retries {
                        hint_text = Some(format!(
                            "Tool '{}' has failed {} consecutive times. Consider an alternative approach or different parameters.",
                            tool_name, count
                        ));
                        break;
                    }
                }
                if let Some(hint) = hint_text {
                    corrected_blocks.push(ContentBlockInput::Text { text: hint });
                }

                // Add tool results as a user message
                messages.push(ChatMessage::new(
                    orka_llm::client::Role::User,
                    ChatContent::Blocks(corrected_blocks),
                ));

                // R1.4: Emit iteration-level event
                self.event_sink
                    .emit(DomainEvent::new(DomainEventKind::AgentIteration {
                        message_id: envelope.id,
                        iteration,
                        tool_count: tool_calls.len(),
                        tokens_used: iteration_tokens,
                        elapsed_ms: iteration_start.elapsed().as_millis() as u64,
                    }))
                    .await;

                if iteration == max_iterations - 1 {
                    warn!(max_iterations, "agent loop reached max iterations");
                    final_text =
                        "I reached the maximum number of reasoning steps. Here's what I have so far."
                            .to_string();
                }
            }

            // Emit Done chunk to signal stream completion
            self.stream_registry.send(StreamChunk::new(
                envelope.session_id,
                envelope.channel.clone(),
                Some(envelope.id),
                StreamChunkKind::Done,
            ));

            // Always persist conversation history, even if final_text is empty
            // (e.g. when the LLM stream fails mid-turn, tool calls should still be saved).
            self.save_conversation_history(
                messages,
                max_entries,
                summarization_model.as_deref(),
                conversation_summary.as_deref(),
                &memory_key,
                &summary_key,
                session_tokens,
                &token_key,
            )
            .await;

            if !final_text.is_empty() {
                // Post-handler experience reflection (async, non-blocking for user response)
                if let (Some(exp), Some(mut tc)) = (&self.experience, trajectory_collector.take()) {
                    tc.set_response(final_text.clone());
                    let trajectory = tc.finish();
                    let exp = exp.clone();
                    let event_sink = self.event_sink.clone();
                    let skills = self.skills.clone();
                    let session_id = envelope.session_id;
                    let trajectory_id = trajectory.id.clone();
                    tokio::spawn(async move {
                        // Persist trajectory for offline distillation
                        if let Err(e) = exp.record_trajectory(&trajectory).await {
                            warn!(%e, "failed to record trajectory");
                        } else {
                            event_sink
                                .emit(DomainEvent::new(DomainEventKind::TrajectoryRecorded {
                                    session_id,
                                    trajectory_id: trajectory_id.clone(),
                                }))
                                .await;
                        }
                        match exp.maybe_reflect(&trajectory).await {
                            Ok(result) => {
                                // Apply structural actions (disable skills with environmental failures)
                                for action in &result.actions {
                                    match action {
                                        orka_experience::StructuralAction::DisableSkill {
                                            skill_name,
                                            reason,
                                        } => {
                                            skills.force_open(skill_name);
                                            warn!(skill = %skill_name, %reason, "skill disabled by experience feedback");
                                            event_sink
                                                .emit(DomainEvent::new(
                                                    DomainEventKind::SkillDisabled {
                                                        skill_name: skill_name.clone(),
                                                        reason: reason.clone(),
                                                        source: "experience_feedback".into(),
                                                    },
                                                ))
                                                .await;
                                        }
                                    }
                                }
                                if result.principles_created > 0 {
                                    event_sink
                                        .emit(DomainEvent::new(
                                            DomainEventKind::ReflectionCompleted {
                                                session_id,
                                                principles_created: result.principles_created,
                                                trajectory_id,
                                            },
                                        ))
                                        .await;
                                }
                            }
                            Err(e) => {
                                warn!(%e, "experience reflection failed");
                            }
                        }
                    });
                }

                // Apply output guardrail
                let final_text = if let Some(ref guardrail) = self.guardrail {
                    match guardrail.check_output(&final_text, session).await? {
                        orka_core::traits::GuardrailDecision::Allow => final_text,
                        orka_core::traits::GuardrailDecision::Block(reason) => {
                            format!("I generated a response but it was filtered: {reason}")
                        }
                        orka_core::traits::GuardrailDecision::Modify(filtered) => filtered,
                        other => {
                            tracing::warn!(?other, "unhandled guardrail decision, passing through");
                            final_text
                        }
                    }
                } else {
                    final_text
                };

                return Ok(vec![self.make_reply(envelope, final_text)]);
            }
        }

        // No LLM configured — tell the user how to fix it
        let reply = format!(
            "[{soul_name}] No LLM provider is configured. To enable AI responses:\n\
             \n\
             1. Set the ANTHROPIC_API_KEY (or OPENAI_API_KEY) environment variable, or\n\
             2. Store the key in the secret manager under the name in orka.toml\n\
             \n\
             Session: {}\nYour message was: {text}",
            session.id
        );
        Ok(vec![self.make_reply(envelope, reply)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orka_core::SessionId;
    use orka_core::testing::{InMemoryEventSink, InMemoryMemoryStore, InMemorySecretManager};

    use orka_workspace::WorkspaceLoader;
    use orka_workspace::config::SoulFrontmatter;
    use orka_workspace::parse::Document;
    use orka_workspace::state::WorkspaceState;

    async fn test_workspace_registry(name: Option<&str>, body: &str) -> Arc<WorkspaceRegistry> {
        let loader = Arc::new(WorkspaceLoader::new("."));
        let state = WorkspaceState {
            soul: Some(Document {
                frontmatter: SoulFrontmatter {
                    name: name.map(|s| s.to_string()),
                    ..Default::default()
                },
                body: body.to_string(),
            }),
            ..Default::default()
        };
        let state_lock = loader.state();
        *state_lock.write().await = state;

        let mut registry = WorkspaceRegistry::new("default".into());
        registry.register("default".into(), loader);
        Arc::new(registry)
    }

    fn test_skill_registry() -> Arc<SkillRegistry> {
        let mut reg = SkillRegistry::new();
        reg.register(Arc::new(orka_skills::EchoSkill));
        Arc::new(reg)
    }

    fn test_handler(ws_registry: Arc<WorkspaceRegistry>) -> WorkspaceHandler {
        test_handler_with_disabled(ws_registry, HashSet::new())
    }

    fn test_handler_with_disabled(
        ws_registry: Arc<WorkspaceRegistry>,
        disabled: HashSet<String>,
    ) -> WorkspaceHandler {
        let skills = test_skill_registry();
        let memory: Arc<dyn orka_core::traits::MemoryStore> = Arc::new(InMemoryMemoryStore::new());
        let secrets: Arc<dyn orka_core::traits::SecretManager> =
            Arc::new(InMemorySecretManager::new());

        let agent_config = AgentConfig::default();

        let mut commands = CommandRegistry::new();
        crate::commands::register_all(
            &mut commands,
            skills.clone(),
            memory.clone(),
            secrets.clone(),
            ws_registry.clone(),
            &agent_config,
            None,
        );
        let commands = Arc::new(commands);

        WorkspaceHandler::new(
            ws_registry,
            skills,
            memory,
            secrets,
            None,
            Arc::new(InMemoryEventSink::new()),
            WorkspaceHandlerConfig {
                agent_config,
                disabled_tools: disabled,
                default_context_window: 128_000,
            },
            None,
            commands,
            StreamRegistry::new(),
            None,
        )
    }

    #[tokio::test]
    async fn soul_name_in_reply() {
        let state = test_workspace_registry(Some("TestBot"), "I am a test bot.").await;
        let handler = test_handler(state);

        let session = Session::new("custom", "user1");
        let envelope = Envelope::text("custom", SessionId::new(), "hello");

        let replies = handler.handle(&envelope, &session).await.unwrap();
        assert_eq!(replies.len(), 1);
        match &replies[0].payload {
            Payload::Text(t) => {
                assert!(t.contains("[TestBot]"));
                assert!(t.contains("No LLM provider is configured"));
                assert!(t.contains("hello"));
            }
            other => panic!("expected text payload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn skill_invocation() {
        let state = test_workspace_registry(Some("Bot"), "").await;
        let handler = test_handler(state);

        let session = Session::new("custom", "user1");
        let envelope = Envelope::text("custom", SessionId::new(), "/skill echo greeting=world");

        let replies = handler.handle(&envelope, &session).await.unwrap();
        assert_eq!(replies.len(), 1);
        match &replies[0].payload {
            Payload::Text(t) => {
                assert!(t.contains("[echo]"));
                assert!(t.contains("greeting"));
                assert!(t.contains("world"));
            }
            other => panic!("expected text payload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unknown_skill_error() {
        let state = test_workspace_registry(Some("Bot"), "").await;
        let handler = test_handler(state);

        let session = Session::new("custom", "user1");
        let envelope = Envelope::text("custom", SessionId::new(), "/skill nonexistent");

        let replies = handler.handle(&envelope, &session).await.unwrap();
        assert_eq!(replies.len(), 1);
        match &replies[0].payload {
            Payload::Text(t) => {
                assert!(t.contains("Unknown skill: nonexistent"));
                assert!(t.contains("echo")); // available skills listed
            }
            other => panic!("expected text payload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_text_rejection() {
        let state = test_workspace_registry(Some("Bot"), "").await;
        let handler = test_handler(state);

        let session = Session::new("custom", "user1");
        let mut envelope = Envelope::text("custom", SessionId::new(), "");
        envelope.payload =
            orka_core::Payload::Event(orka_core::EventPayload::new("test", Default::default()));

        let replies = handler.handle(&envelope, &session).await.unwrap();
        assert_eq!(replies.len(), 1);
        match &replies[0].payload {
            Payload::Text(t) => assert!(t.contains("only process text")),
            other => panic!("expected text payload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn command_payload_routed_as_slash_command() {
        let state = test_workspace_registry(Some("Bot"), "").await;
        let handler = test_handler(state);

        let session = Session::new("custom", "user1");
        let mut envelope = Envelope::text("custom", SessionId::new(), "");
        envelope.payload = Payload::Command(orka_core::CommandPayload::new("test", HashMap::new()));

        let replies = handler.handle(&envelope, &session).await.unwrap();
        assert_eq!(replies.len(), 1);
        // /test is not a registered command, so it returns the "Unknown command" error
        match &replies[0].payload {
            Payload::Text(t) => assert!(t.contains("Unknown command")),
            other => panic!("expected text payload, got {other:?}"),
        }
    }

    #[test]
    fn format_datetime_utc_default() {
        let result = format_current_datetime(None);
        assert!(result.starts_with("Current date and time:"));
        assert!(result.contains("UTC"));
    }

    #[test]
    fn format_datetime_with_valid_timezone() {
        let result = format_current_datetime(Some("Europe/Rome"));
        assert!(result.contains("Europe/Rome"));
    }

    #[test]
    fn format_datetime_invalid_falls_back_to_utc() {
        let result = format_current_datetime(Some("Invalid/Tz"));
        assert!(result.contains("UTC"));
    }

    #[tokio::test]
    async fn build_tool_definitions_includes_registered_skills() {
        let state = test_workspace_registry(Some("Bot"), "").await;
        let handler = test_handler(state);

        let defs = handler.build_tool_definitions();
        // "echo" + 2 built-in workspace tools
        assert_eq!(defs.len(), 3);
        assert_eq!(defs[0].name, "echo");
        assert_eq!(defs[1].name, "workspace_info");
        assert_eq!(defs[2].name, "workspace_switch");
    }

    #[tokio::test]
    async fn build_tool_definitions_excludes_disabled() {
        let state = test_workspace_registry(Some("Bot"), "").await;
        let disabled: HashSet<String> = ["echo".to_string()].into_iter().collect();
        let handler = test_handler_with_disabled(state, disabled);

        let defs = handler.build_tool_definitions();
        // Only the 2 built-in workspace tools remain
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].name, "workspace_info");
        assert_eq!(defs[1].name, "workspace_switch");
    }

    #[test]
    fn system_prompt_includes_workspace_awareness() {
        let prompt = WorkspaceHandler::build_system_prompt(
            "TestBot",
            "I am helpful.",
            "",
            None,
            "main",
            &["main", "support"],
            "",
            None,
            None,
        );
        assert!(prompt.contains("You are TestBot."));
        assert!(prompt.contains("workspace \"main\""));
        assert!(prompt.contains("main, support"));
        assert!(prompt.contains("workspace_info"));
        assert!(prompt.contains("workspace_switch"));
    }

    #[test]
    fn system_prompt_includes_principles_when_provided() {
        let principles = "\n\n## Learned Principles\n\n1. [DO] Use web_search for current info.\n";
        let prompt = WorkspaceHandler::build_system_prompt(
            "TestBot",
            "",
            "",
            None,
            "main",
            &["main"],
            principles,
            None,
            None,
        );
        assert!(prompt.contains("Learned Principles"));
        assert!(prompt.contains("Use web_search"));
    }

    #[test]
    fn system_prompt_omits_principles_when_empty() {
        let prompt = WorkspaceHandler::build_system_prompt(
            "TestBot",
            "",
            "",
            None,
            "main",
            &["main"],
            "",
            None,
            None,
        );
        assert!(!prompt.contains("Learned Principles"));
    }

    #[test]
    fn system_prompt_injects_summary_in_system_context_not_as_user_message() {
        let summary = "The user asked about the weather and I explained it was sunny.";
        let prompt = WorkspaceHandler::build_system_prompt(
            "TestBot",
            "",
            "",
            None,
            "main",
            &["main"],
            "",
            Some(summary),
            None,
        );
        assert!(
            prompt.contains("## Prior Conversation Context"),
            "summary section header missing"
        );
        assert!(
            prompt.contains(summary),
            "summary text missing from system prompt"
        );
    }

    #[test]
    fn system_prompt_omits_summary_section_when_none() {
        let prompt = WorkspaceHandler::build_system_prompt(
            "TestBot",
            "",
            "",
            None,
            "main",
            &["main"],
            "",
            None,
            None,
        );
        assert!(!prompt.contains("## Prior Conversation Context"));
    }

    async fn multi_workspace_registry() -> Arc<WorkspaceRegistry> {
        use orka_workspace::config::SoulFrontmatter;
        use orka_workspace::parse::Document;
        use orka_workspace::state::WorkspaceState;

        let mut registry = WorkspaceRegistry::new("main".into());

        let main_loader = Arc::new(WorkspaceLoader::new("."));
        *main_loader.state().write().await = WorkspaceState {
            soul: Some(Document {
                frontmatter: SoulFrontmatter {
                    name: Some("MainBot".into()),
                    ..Default::default()
                },
                body: "Main persona".into(),
            }),
            ..Default::default()
        };
        registry.register("main".into(), main_loader);

        let support_loader = Arc::new(WorkspaceLoader::new("workspaces/support"));
        *support_loader.state().write().await = WorkspaceState {
            soul: Some(Document {
                frontmatter: SoulFrontmatter {
                    name: Some("SupportBot".into()),
                    ..Default::default()
                },
                body: "Support persona".into(),
            }),
            ..Default::default()
        };
        registry.register("support".into(), support_loader);

        Arc::new(registry)
    }

    #[tokio::test]
    async fn workspace_info_returns_all_workspaces() {
        let registry = multi_workspace_registry().await;
        let handler = test_handler(registry);

        let session = Session::new("custom", "user1");
        let call = ToolCall::new("call_1", "workspace_info", serde_json::json!({}));

        let result = handler.handle_builtin_tool(&call, &session, "main").await;
        assert!(result.is_some());
        let (content, is_error) = result.unwrap();
        assert!(!is_error);
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["current_workspace"], "main");
        let workspaces = parsed["workspaces"].as_array().unwrap();
        assert_eq!(workspaces.len(), 2);
    }

    #[tokio::test]
    async fn workspace_switch_stores_override() {
        let registry = multi_workspace_registry().await;
        let handler = test_handler(registry);

        let session = Session::new("custom", "user1");
        let call = ToolCall::new(
            "call_1",
            "workspace_switch",
            serde_json::json!({"name": "support"}),
        );

        let result = handler.handle_builtin_tool(&call, &session, "main").await;
        assert!(result.is_some());
        let (content, is_error) = result.unwrap();
        assert!(!is_error);
        assert!(content.contains("support"));

        // Verify the override was stored in memory
        let override_key = format!("workspace_override:{}", session.id);
        let entry = handler.memory.recall(&override_key).await.unwrap().unwrap();
        assert_eq!(
            entry.value.get("workspace_name").unwrap().as_str().unwrap(),
            "support"
        );
    }

    #[tokio::test]
    async fn workspace_switch_rejects_unknown() {
        let registry = multi_workspace_registry().await;
        let handler = test_handler(registry);

        let session = Session::new("custom", "user1");
        let call = ToolCall::new(
            "call_1",
            "workspace_switch",
            serde_json::json!({"name": "nonexistent"}),
        );

        let result = handler.handle_builtin_tool(&call, &session, "main").await;
        assert!(result.is_some());
        let (content, is_error) = result.unwrap();
        assert!(is_error);
        assert!(content.contains("not found"));
    }

    #[tokio::test]
    async fn non_builtin_tool_returns_none() {
        let registry = multi_workspace_registry().await;
        let handler = test_handler(registry);

        let session = Session::new("custom", "user1");
        let call = ToolCall::new("call_1", "echo", serde_json::json!({"greeting": "hello"}));

        let result = handler.handle_builtin_tool(&call, &session, "main").await;
        assert!(result.is_none());
    }

    // --- Pure helper tests ---

    #[test]
    fn truncate_short_content_unchanged() {
        let s = "short content";
        assert_eq!(truncate_tool_result(s, 100), s);
    }

    #[test]
    fn truncate_long_content_shows_boundary() {
        let s = "a".repeat(1000);
        let result = truncate_tool_result(&s, 100);
        assert!(result.contains("[truncated"));
        assert!(result.contains("1000 total"));
        assert!(result.len() < s.len());
    }

    #[test]
    fn truncate_multibyte_respects_char_boundary() {
        // 4-byte emoji repeated; truncation must not panic
        let s = "🦀".repeat(100);
        let result = truncate_tool_result(&s, 50);
        assert!(result.contains("[truncated"));
    }

    #[test]
    fn tool_metadata_web_search() {
        let input = serde_json::json!({"query": "rust async"});
        let (cat, summary) = tool_metadata("web_search", &input);
        assert_eq!(cat.as_deref(), Some("search"));
        assert!(summary.unwrap().contains("rust async"));
    }

    #[test]
    fn tool_metadata_http_request() {
        let input = serde_json::json!({"method": "POST", "url": "https://api.example.com"});
        let (cat, summary) = tool_metadata("http_request", &input);
        assert_eq!(cat.as_deref(), Some("http"));
        assert!(summary.unwrap().contains("POST https://api.example.com"));
    }

    #[test]
    fn tool_metadata_unknown_tool() {
        let (cat, summary) = tool_metadata("custom_tool", &serde_json::json!({}));
        assert!(cat.is_none());
        assert!(summary.is_none());
    }

    #[test]
    fn summarize_result_error_truncates() {
        let long_err = "x".repeat(200);
        let result = summarize_result("any_tool", &long_err, true).unwrap();
        assert!(result.len() < 100);
        assert!(result.ends_with('…'));
    }

    #[test]
    fn summarize_result_web_search_counts() {
        let json_array = r#"[{"title":"a"},{"title":"b"}]"#;
        let result = summarize_result("web_search", json_array, false).unwrap();
        assert_eq!(result, "Found 2 results");
    }

    #[test]
    fn build_transcript_formats_roles() {
        let msgs = vec![ChatMessage::user("hello"), ChatMessage::assistant("world")];
        let t = WorkspaceHandler::build_transcript(&msgs);
        assert!(t.contains("user: hello"));
        assert!(t.contains("assistant: world"));
    }

    #[test]
    fn fallback_summary_extracts_user_bullets() {
        let msgs = vec![
            ChatMessage::user("first question"),
            ChatMessage::assistant("response"),
            ChatMessage::user("second question"),
        ];
        let s = WorkspaceHandler::fallback_summary(&msgs);
        assert!(s.contains("- first question"));
        assert!(s.contains("- second question"));
        // Assistant messages should not appear as bullets
        assert!(!s.contains("- response"));
    }
}
