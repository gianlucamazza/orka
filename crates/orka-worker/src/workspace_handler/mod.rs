mod conversation;
mod tool_exec;
mod tool_meta;

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Instant,
};

use async_trait::async_trait;
use orka_core::{
    CommandArgs, DomainEvent, DomainEventKind, Envelope, MemoryEntry, OutboundMessage, Payload,
    Result, Session, SessionId,
    config::AgentConfig,
    traits::{EventSink, Guardrail, MemoryStore, SecretManager},
};
use orka_experience::ExperienceService;
use orka_llm::{
    client::{
        ChatContent, ChatMessage, CompletionOptions, ContentBlock, ContentBlockInput, LlmClient,
        StopReason, ToolCall, ToolDefinition,
    },
    context::{
        TokenizerHint, available_history_budget_with_hint, estimate_message_tokens_with_hint,
        truncate_history_with_hint,
    },
};
use orka_prompts::pipeline::{BuildContext, PipelineConfig, SystemPromptPipeline};
use orka_skills::SkillRegistry;
use orka_workspace::WorkspaceRegistry;
use tracing::{Instrument, debug, info, info_span, warn};

use crate::{
    commands::CommandRegistry,
    handler::AgentHandler,
    stream::{StreamChunk, StreamChunkKind, StreamRegistry},
};

/// Configuration parameters for [`WorkspaceHandler`], grouped to reduce
/// constructor arguments.
pub struct WorkspaceHandlerConfig {
    /// LLM and agent tuning parameters.
    pub agent_config: AgentConfig,
    /// Tool names that should never be offered to the LLM.
    pub disabled_tools: HashSet<String>,
    /// Fallback context window size when the model info is unavailable.
    pub default_context_window: u32,
}

/// Sliding-window rate limiter for slash commands, keyed by `(SessionId,
/// command_name)`.
///
/// Stores `(window_start, call_count)` per session+command pair and resets the
/// count after `RATE_WINDOW_SECS` seconds.
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

    /// Returns `true` if the command is allowed, `false` if the limit is
    /// exceeded.
    fn check_and_record(&self, session_id: SessionId, command: &str) -> bool {
        let mut guard = match self.state.lock() {
            Ok(g) => g,
            Err(_) => {
                warn!("CommandRateLimiter mutex poisoned, allowing command to fail-open");
                return true;
            }
        };
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

/// LLM-powered agent handler with tool-use loops, guardrails, and experience
/// learning.
pub struct WorkspaceHandler {
    pub(super) workspace_registry: Arc<WorkspaceRegistry>,
    pub(super) skills: Arc<SkillRegistry>,
    pub(super) memory: Arc<dyn MemoryStore>,
    pub(super) secrets: Arc<dyn SecretManager>,
    pub(super) llm: Option<Arc<dyn LlmClient>>,
    pub(super) event_sink: Arc<dyn EventSink>,
    pub(super) agent_config: AgentConfig,
    pub(super) disabled_tools: HashSet<String>,
    pub(super) default_context_window: u32,
    pub(super) guardrail: Option<Arc<dyn Guardrail>>,
    pub(super) commands: Arc<CommandRegistry>,
    pub(super) stream_registry: StreamRegistry,
    pub(super) experience: Option<Arc<ExperienceService>>,
    /// Per-session rate limiter for slash commands (10 per minute by default).
    command_rate_limiter: CommandRateLimiter,
    /// Shared cancellation tokens from the worker pool (used by `/cancel`).
    pub(super) session_cancel_tokens: Option<crate::SessionCancelTokens>,
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

    /// Build LLM tool definitions from skill registry, excluding disabled
    /// tools. Also appends built-in workspace management tools.
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

    /// Resolve workspace from the registry by name. Falls back to default if
    /// not found.
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
            .unwrap_or_else(|| self.agent_config.name.clone());
        let soul_body = state
            .soul
            .as_ref()
            .map(|doc| doc.body.clone())
            .unwrap_or_default();
        let tools_body = state.tools_body.clone().unwrap_or_default();
        (soul_name, soul_body, tools_body)
    }

    /// Resolve workspace from inline CLI content (raw SOUL.md/TOOLS.md
    /// strings). Falls back to the default workspace for any missing piece.
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
                        .unwrap_or_else(|| self.agent_config.name.clone()),
                    doc.body,
                ),
                Err(e) => {
                    warn!(%e, "failed to parse workspace override SOUL.md, falling back");
                    (self.agent_config.name.clone(), raw.to_string())
                }
            }
        } else {
            let state = self.workspace_registry.default_state();
            let state = state.read().await;
            let name = state
                .soul
                .as_ref()
                .and_then(|doc| doc.frontmatter.name.clone())
                .unwrap_or_else(|| self.agent_config.name.clone());
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

    /// Handle a built-in workspace tool call. Returns `Some(result)` if the
    /// tool was handled, or `None` if it should be dispatched to the skill
    /// registry.
    pub(super) async fn handle_builtin_tool(
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

    /// Compact oversized tool results — delegates to the shared
    /// [`crate::history`] module.
    fn compact_tool_results(messages: Vec<ChatMessage>, max_chars: usize) -> Vec<ChatMessage> {
        crate::history::compact_tool_results(messages, max_chars)
    }

    /// Persist conversation history and token usage to the memory store.
    ///
    /// When the history exceeds `max_entries` the oldest messages are
    /// summarised using incremental rolling summarisation.  The resulting
    /// summary text is stored separately under `summary_key` and injected
    /// into the system prompt on the next
    /// turn via [`Self::build_system_prompt`].
    #[allow(clippy::too_many_arguments)]
    async fn save_conversation_history(
        &self,
        messages: Vec<ChatMessage>,
        max_entries: usize,
        _summarization_model: Option<&str>,
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
                Self::summarize_messages(llm, old_messages, None, existing_summary).await
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

        // Dispatch ALL slash commands before the guardrail.  Commands are trusted
        // internal handlers — there is no reason to run a guardrail check on
        // them.
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
        //   1. Per-session override from MemoryStore (CLI inline content or named
        //      workspace)
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
        let max_iterations = agent.max_iterations;
        let context_window = self.default_context_window;

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
                        exp.format_principles(&principles).await
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

            // Get workspace info for prompt building
            let user_cwd = envelope
                .metadata
                .get("workspace:cwd")
                .and_then(|v| v.as_str());

            // Get template registry from workspace state
            let state_lock = self.workspace_registry.default_state();
            let state = state_lock.read().await;
            let template_registry = state.templates.clone();
            drop(state);

            // Build system prompt using the configurable pipeline
            let pipeline_config = PipelineConfig::default();
            let pipeline = SystemPromptPipeline::from_config(&pipeline_config);

            // Parse principles from JSON string if available
            let principles = if !principles_section.is_empty() {
                // Simple parsing: extract principle items from the formatted section
                principles_section
                    .lines()
                    .filter(|line| line.contains(". [") && line.contains("] "))
                    .enumerate()
                    .map(|(i, line)| {
                        let kind = if line.contains("[AVOID]") {
                            "avoid"
                        } else {
                            "do"
                        };
                        let text = line.split("] ").nth(1).unwrap_or("").to_string();
                        serde_json::json!({
                            "index": i + 1,
                            "kind": kind,
                            "text": text,
                        })
                    })
                    .collect()
            } else {
                vec![]
            };

            // Build context
            let available_workspaces: Vec<String> = available_ws_refs
                .into_iter()
                .map(|s| s.to_string())
                .collect();

            let mut ctx = BuildContext::new(&soul_name)
                .with_persona(&soul_body)
                .with_tool_instructions(&tools_body)
                .with_workspace(&resolved_workspace, available_workspaces)
                .with_principles(principles)
                .with_config(pipeline_config);

            if let Some(cwd) = user_cwd {
                ctx = ctx.with_cwd(cwd);
            }

            if let Some(summary) = conversation_summary.as_deref() {
                ctx = ctx.with_summary(summary);
            }

            // Add shell commands as dynamic section if present
            if let Some(shell_ctx) = envelope
                .metadata
                .get("shell:recent_commands")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                ctx = ctx.with_dynamic_section(
                    "shell_commands",
                    format!("## Recent local shell commands\n{shell_ctx}"),
                );
            }

            if let Some(registry) = template_registry {
                ctx = ctx.with_templates(registry);
            }

            let system_prompt = match pipeline.build(&ctx).await {
                Ok(prompt) => prompt,
                Err(e) => {
                    warn!(error = %e, "failed to build system prompt with pipeline, using fallback");
                    // Fallback to basic format
                    format!("You are {soul_name}.\n\n{soul_body}")
                }
            };

            let mut options = CompletionOptions::default();
            options.model = Some(soul_model.clone());
            options.max_tokens = Some(soul_max_tokens);

            // Agent loop: call LLM, execute tool calls, feed results back
            let mut final_text = String::new();
            let llm_model = soul_model.clone();
            let max_tool_retries = 3; // Default max retries
            // Track per-tool-name consecutive error counts for self-correction
            let mut tool_error_counts: HashMap<String, u32> = HashMap::new();
            for iteration in 0..max_iterations {
                // Check for cancellation between iterations.
                if let Some(ref tokens) = self.session_cancel_tokens {
                    let cancelled = match tokens.lock() {
                        Ok(t) => t
                            .get(&envelope.session_id)
                            .map(|t| t.is_cancelled())
                            .unwrap_or(false),
                        Err(_) => {
                            warn!("session_cancel_tokens lock poisoned, assuming not cancelled");
                            false
                        }
                    };
                    if cancelled {
                        info!(session_id = %envelope.session_id, "operation cancelled by user");
                        final_text = "Operation cancelled.".to_string();
                        break;
                    }
                }

                let iteration_start = std::time::Instant::now();

                // Pre-flight truncation: ensure messages fit context window
                let output_budget = soul_max_tokens;
                let hint = TokenizerHint::from_model(Some(soul_model.as_str()));
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
                let completion = match orka_llm::consume_stream(
                    stream,
                    &envelope.session_id,
                    &self.stream_registry,
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
                        provider: orka_llm::infer_provider(&llm_model),
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

                // Check token budget (simplified - no per-session budget in new config)
                // Token tracking still happens but no hard limit enforced

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
            let max_entries = 50; // Default max history entries
            self.save_conversation_history(
                messages,
                max_entries,
                None, // summarization_model removed from config
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
                                // Apply structural actions (disable skills with environmental
                                // failures)
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
    use orka_core::{
        SessionId,
        testing::{InMemoryEventSink, InMemoryMemoryStore, InMemorySecretManager},
    };
    use orka_workspace::{
        WorkspaceLoader, config::SoulFrontmatter, parse::Document, state::WorkspaceState,
    };

    use super::*;

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

    // Note: I test per format_current_datetime sono stati rimossi
    // La formattazione datetime è ora gestita dalla pipeline in orka-prompts

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

    // Note: I test per system prompt sono stati spostati in orka-prompts
    // dove vengono testati con la pipeline template-based

    async fn multi_workspace_registry() -> Arc<WorkspaceRegistry> {
        use orka_workspace::{config::SoulFrontmatter, parse::Document, state::WorkspaceState};

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

    #[test]
    fn tool_metadata_web_search() {
        let input = serde_json::json!({"query": "rust async"});
        let (cat, summary) = tool_meta::tool_metadata("web_search", &input);
        assert_eq!(cat.as_deref(), Some("search"));
        assert!(summary.unwrap().contains("rust async"));
    }

    #[test]
    fn tool_metadata_http_request() {
        let input = serde_json::json!({"method": "POST", "url": "https://api.example.com"});
        let (cat, summary) = tool_meta::tool_metadata("http_request", &input);
        assert_eq!(cat.as_deref(), Some("http"));
        assert!(summary.unwrap().contains("POST https://api.example.com"));
    }

    #[test]
    fn tool_metadata_unknown_tool() {
        let (cat, summary) = tool_meta::tool_metadata("custom_tool", &serde_json::json!({}));
        assert!(cat.is_none());
        assert!(summary.is_none());
    }

    #[test]
    fn summarize_result_error_truncates() {
        let long_err = "x".repeat(200);
        let result = tool_meta::summarize_result("any_tool", &long_err, true).unwrap();
        assert!(result.len() < 100);
        assert!(result.ends_with('…'));
    }

    #[test]
    fn summarize_result_web_search_counts() {
        let json_array = r#"[{"title":"a"},{"title":"b"}]"#;
        let result = tool_meta::summarize_result("web_search", json_array, false).unwrap();
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
