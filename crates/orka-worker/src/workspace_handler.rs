use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use orka_core::config::AgentConfig;
use orka_core::traits::{EventSink, Guardrail, MemoryStore, SecretManager};
use orka_core::{
    DomainEvent, DomainEventKind, Envelope, MemoryEntry, MessageId, OutboundMessage, Payload,
    Result, Session, SessionId, SkillInput,
};

use orka_experience::ExperienceService;
use orka_llm::client::{
    ChatContent, ChatMessageExt, CompletionOptions, CompletionResponse, ContentBlock,
    ContentBlockInput, LlmClient, LlmToolStream, StopReason, StreamEvent, ToolCall, ToolDefinition,
    Usage,
};
use orka_llm::context::{available_history_budget, truncate_history};
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
    let truncated = &content[..max_chars];
    format!(
        "{truncated}\n\n[truncated, showing first {max_chars} chars of {} total]",
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
    pub agent_config: AgentConfig,
    pub disabled_tools: HashSet<String>,
    pub default_context_window: u32,
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
        }
    }

    fn make_reply(&self, envelope: &Envelope, text: String) -> OutboundMessage {
        let mut msg = OutboundMessage::text(
            envelope.channel.clone(),
            envelope.session_id,
            text,
            Some(envelope.id),
        );
        msg.metadata = envelope.metadata.clone();
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
    fn build_system_prompt(
        soul_name: &str,
        soul_body: &str,
        tools_body: &str,
        timezone: Option<&str>,
        workspace_name: &str,
        available_workspaces: &[&str],
        principles_section: &str,
    ) -> String {
        let mut prompt = if soul_body.is_empty() {
            format!("You are {soul_name}.")
        } else {
            format!("You are {soul_name}.\n\n{soul_body}")
        };

        let now_str = format_current_datetime(timezone);
        prompt.push_str("\n\n");
        prompt.push_str(&now_str);

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
    ) -> Vec<ContentBlockInput> {
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

            info!(skill = %call.name, id = %call.id, "invoking skill via tool call");

            self.event_sink
                .emit(DomainEvent::new(DomainEventKind::SkillInvoked {
                    skill_name: call.name.clone(),
                    message_id: envelope.id,
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
            let call_input = call.input.clone();
            let skills = self.skills.clone();
            let event_sink = self.event_sink.clone();
            let message_id = envelope.id;
            let secrets = self.secrets.clone();
            let skill_timeout =
                std::time::Duration::from_secs(self.agent_config.skill_timeout_secs);
            let max_result_chars = self.agent_config.max_tool_result_chars;

            join_set.spawn(async move {
                let args: HashMap<String, serde_json::Value> = match call_input {
                    serde_json::Value::Object(map) => map.into_iter().collect(),
                    _ => HashMap::new(),
                };

                let start = std::time::Instant::now();
                let skill_input = SkillInput::new(args).with_context(orka_core::SkillContext::new(
                    secrets,
                    Some(event_sink.clone()),
                ));
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

                let (content, is_error) = match &result {
                    Ok(output) => {
                        let raw = output.data.to_string();
                        (truncate_tool_result(&raw, max_result_chars), false)
                    }
                    Err(e) => (format!("Error: {e}"), true),
                };

                event_sink
                    .emit(DomainEvent::new(DomainEventKind::SkillCompleted {
                        skill_name: call_name,
                        message_id,
                        duration_ms,
                        success: !is_error,
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
                )
            });
        }

        // Collect results and emit ToolExecEnd chunks
        let mut results_map: HashMap<String, (String, bool)> = HashMap::new();
        while let Some(res) = join_set.join_next().await {
            if let Ok((call_id, content, is_error, duration_ms, error_msg, result_summary)) = res {
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
                results_map.insert(call_id, (content, is_error));
            }
        }

        // Build result blocks in original order, merging built-in and skill results
        tool_calls
            .iter()
            .enumerate()
            .map(|(idx, call)| {
                let (content, is_error) = if let Some(result) = builtin_results.remove(&idx) {
                    result
                } else {
                    results_map
                        .remove(&call.id)
                        .unwrap_or_else(|| ("Error: task failed".to_string(), true))
                };
                ContentBlockInput::ToolResult {
                    tool_use_id: call.id.clone(),
                    content,
                    is_error,
                }
            })
            .collect()
    }

    /// Persist conversation history and token usage to the memory store.
    #[allow(clippy::too_many_arguments)]
    async fn save_conversation_history(
        &self,
        messages: Vec<ChatMessageExt>,
        max_entries: usize,
        summarization_threshold: usize,
        summarization_model: Option<&str>,
        memory_key: &str,
        session_tokens: u64,
        token_key: &str,
    ) {
        let history_to_save = if messages.len() > max_entries {
            if let (Some(llm), true) = (&self.llm, messages.len() > summarization_threshold) {
                let split_point = messages.len() - max_entries;
                let old_messages = &messages[..split_point];

                let summary_text =
                    Self::summarize_messages(llm, old_messages, summarization_model).await;
                let mut condensed = vec![ChatMessageExt::text(
                    "user",
                    format!("[Previous conversation summary: {}]", summary_text),
                )];
                condensed.extend_from_slice(&messages[split_point..]);
                condensed
            } else {
                messages[messages.len() - max_entries..].to_vec()
            }
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

    async fn summarize_messages(
        llm: &Arc<dyn LlmClient>,
        messages: &[ChatMessageExt],
        model: Option<&str>,
    ) -> String {
        use orka_llm::client::ChatMessage;

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
                                parts.push(format!("[result: {content}]"))
                            }
                            _ => {
                                continue;
                            }
                        }
                    }
                    if parts.is_empty() {
                        "[tool interaction]".to_string()
                    } else {
                        parts.join(" ")
                    }
                }
                _ => {
                    continue;
                }
            };
            transcript.push_str(&format!("{}: {}\n", msg.role, text));
        }

        let summary_prompt = vec![ChatMessage::new(
            "user",
            format!(
                "Summarize the following conversation concisely, preserving key facts, decisions, and context:\n\n{}",
                transcript
            ),
        )];

        let mut options = CompletionOptions::default();
        options.model = model.map(|s| s.to_string());
        options.max_tokens = Some(512);

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
                tracing::warn!(%e, "failed to summarize conversation, using truncation");
                format!("[{} messages truncated]", messages.len())
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
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut current_tool_id: Option<String> = None;
        let mut current_tool_name: Option<String> = None;
        let mut current_tool_input = String::new();
        let mut usage = Usage::default();
        let mut stop_reason = None;

        while let Some(event) = stream.next().await {
            let event = event?;
            match event {
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
                _ => {}
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
        let text = match &envelope.payload {
            Payload::Text(t) => t.clone(),
            _ => {
                return Ok(vec![self.make_reply(
                    envelope,
                    "Sorry, I can only process text messages.".into(),
                )]);
            }
        };

        // 3-tier workspace resolution:
        //   1. Per-session override from MemoryStore (CLI inline content or named workspace)
        //   2. Adapter-level workspace binding (workspace:name in envelope metadata)
        //   3. Default workspace from registry
        let override_key = format!("workspace_override:{}", session.id);

        // Persist CLI metadata overrides into MemoryStore for session stickiness
        let has_ws_meta = envelope.metadata.contains_key("workspace:soul")
            || envelope.metadata.contains_key("workspace:tools");

        if has_ws_meta {
            let override_val = serde_json::json!({
                "soul": envelope.metadata.get("workspace:soul").and_then(|v| v.as_str()),
                "tools": envelope.metadata.get("workspace:tools").and_then(|v| v.as_str()),
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
        let summarization_threshold = agent.summarization_threshold.unwrap_or(max_entries * 2);
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

        // Check for slash commands: /command [args...]
        if let Some(cmd) = orka_core::parse_slash_command(&text) {
            if cmd.name == "help" {
                // /help uses the registry to show all commands
                return Ok(vec![self.make_reply(envelope, self.commands.help_text())]);
            }
            if let Some(handler) = self.commands.get(&cmd.name) {
                return handler.execute(&cmd.args, envelope, session).await;
            }
            // Unknown slash command
            let help = self.commands.help_text();
            return Ok(vec![self.make_reply(
                envelope,
                format!("Unknown command: /{}\n\n{help}", cmd.name),
            )]);
        }

        // If LLM is configured, run the agent loop
        if let Some(ref llm) = self.llm {
            let tools = self.build_tool_definitions();

            // Load conversation history and token usage in parallel
            let memory_key = format!("conversation:{}", session.id);
            let token_key = format!("tokens:{}", session.id);
            let (history_result, tokens_result) = tokio::join!(
                self.memory.recall(&memory_key),
                self.memory.recall(&token_key),
            );

            let history: Vec<ChatMessageExt> = match history_result {
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
            messages.push(ChatMessageExt::text("user", text.clone()));

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

            let system_prompt = Self::build_system_prompt(
                &soul_name,
                &soul_body,
                &tools_body,
                soul_timezone.as_deref(),
                &resolved_workspace,
                &available_ws_refs,
                &principles_section,
            );

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
                let iteration_start = std::time::Instant::now();

                // Pre-flight truncation: ensure messages fit context window
                let output_budget = soul_max_tokens.unwrap_or(4096);
                let budget =
                    available_history_budget(context_window, output_budget, &system_prompt, &tools);
                let (truncated, dropped) = truncate_history(messages, budget);
                messages = truncated;
                if dropped > 0 {
                    warn!(
                        dropped,
                        remaining = messages.len(),
                        budget,
                        "truncated history to fit context window"
                    );
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
                        input_tokens: completion.usage.input_tokens,
                        output_tokens: completion.usage.output_tokens,
                        duration_ms: llm_duration_ms,
                        estimated_cost_usd: None,
                    }))
                    .await;

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

                // Collect text and tool calls from response
                let mut response_text = String::new();
                let mut tool_calls = Vec::new();
                for block in &completion.blocks {
                    match block {
                        ContentBlock::Text(t) => response_text.push_str(t),
                        ContentBlock::ToolUse(call) => tool_calls.push(call.clone()),
                        _ => {}
                    }
                }

                // R1.1: Extract and log reasoning text (text before first tool call)
                if !response_text.is_empty() {
                    self.event_sink
                        .emit(DomainEvent::new(DomainEventKind::AgentReasoning {
                            message_id: envelope.id,
                            iteration,
                            reasoning_text: response_text.clone(),
                        }))
                        .await;
                }

                if tool_calls.is_empty() {
                    // No tool calls — final response
                    final_text = response_text;
                    messages.push(ChatMessageExt::text("assistant", final_text.clone()));
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
                    messages.push(ChatMessageExt::new(
                        "assistant",
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
                let result_blocks = self
                    .execute_tool_calls(&tool_calls, envelope, session, &resolved_workspace)
                    .instrument(tool_span)
                    .await;

                // R1.3: Track per-tool error counts and inject self-correction hints
                let mut corrected_blocks = result_blocks;
                for (block, call) in corrected_blocks.iter().zip(tool_calls.iter()) {
                    if let ContentBlockInput::ToolResult { is_error, .. } = block {
                        // Record skill in trajectory collector
                        if let Some(ref mut tc) = trajectory_collector {
                            tc.record_skill(call.name.clone(), 0, !*is_error);
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
                messages.push(ChatMessageExt::new(
                    "user",
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

            if !final_text.is_empty() {
                // Save conversation history and token usage
                self.save_conversation_history(
                    messages,
                    max_entries,
                    summarization_threshold,
                    summarization_model.as_deref(),
                    &memory_key,
                    session_tokens,
                    &token_key,
                )
                .await;

                // Post-handler experience reflection (async, non-blocking for user response)
                if let (Some(exp), Some(mut tc)) = (&self.experience, trajectory_collector.take()) {
                    tc.set_response(final_text.clone());
                    let trajectory = tc.finish();
                    let exp = exp.clone();
                    let event_sink = self.event_sink.clone();
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
                            Ok(count) if count > 0 => {
                                event_sink
                                    .emit(DomainEvent::new(DomainEventKind::ReflectionCompleted {
                                        session_id,
                                        principles_created: count,
                                        trajectory_id,
                                    }))
                                    .await;
                            }
                            Err(e) => {
                                warn!(%e, "experience reflection failed");
                            }
                            _ => {}
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
                        _ => final_text,
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
        envelope.payload = Payload::Command(orka_core::CommandPayload::new("test", HashMap::new()));

        let replies = handler.handle(&envelope, &session).await.unwrap();
        assert_eq!(replies.len(), 1);
        match &replies[0].payload {
            Payload::Text(t) => assert!(t.contains("only process text")),
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
        );
        assert!(prompt.contains("Learned Principles"));
        assert!(prompt.contains("Use web_search"));
    }

    #[test]
    fn system_prompt_omits_principles_when_empty() {
        let prompt =
            WorkspaceHandler::build_system_prompt("TestBot", "", "", None, "main", &["main"], "");
        assert!(!prompt.contains("Learned Principles"));
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
}
