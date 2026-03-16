use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use orka_core::traits::{EventSink, Guardrail, MemoryStore, SecretManager};
use orka_core::{
    DomainEvent, DomainEventKind, Envelope, EventId, MemoryEntry, OutboundMessage, Payload, Result,
    Session, SkillInput,
};
use orka_llm::client::{
    ChatContent, ChatMessageExt, CompletionOptions, ContentBlock, ContentBlockInput, LlmClient,
    StopReason, ToolDefinition,
};
use orka_llm::context::{available_history_budget, truncate_history};
use orka_skills::SkillRegistry;
use orka_workspace::config::ToolEntry;
use orka_workspace::state::WorkspaceState;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::handler::AgentHandler;

const MAX_AGENT_ITERATIONS: usize = 10;

pub struct WorkspaceHandler {
    workspace_state: Arc<RwLock<WorkspaceState>>,
    skills: Arc<SkillRegistry>,
    memory: Arc<dyn MemoryStore>,
    secrets: Arc<dyn SecretManager>,
    llm: Option<Arc<dyn LlmClient>>,
    event_sink: Arc<dyn EventSink>,
    default_context_window: u32,
    guardrail: Option<Arc<dyn Guardrail>>,
}

impl WorkspaceHandler {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workspace_state: Arc<RwLock<WorkspaceState>>,
        skills: Arc<SkillRegistry>,
        memory: Arc<dyn MemoryStore>,
        secrets: Arc<dyn SecretManager>,
        llm: Option<Arc<dyn LlmClient>>,
        event_sink: Arc<dyn EventSink>,
        default_context_window: u32,
        guardrail: Option<Arc<dyn Guardrail>>,
    ) -> Self {
        Self {
            workspace_state,
            skills,
            memory,
            secrets,
            llm,
            event_sink,
            default_context_window,
            guardrail,
        }
    }

    fn make_reply(&self, envelope: &Envelope, text: String) -> OutboundMessage {
        OutboundMessage {
            channel: envelope.channel.clone(),
            session_id: envelope.session_id.clone(),
            payload: Payload::Text(text),
            reply_to: Some(envelope.id.clone()),
            metadata: envelope.metadata.clone(),
        }
    }

    /// Convert workspace TOOLS.md entries to LLM tool definitions,
    /// filtering to only enabled tools that exist in the skill registry.
    fn build_tool_definitions(&self, tool_entries: &[ToolEntry]) -> Vec<ToolDefinition> {
        tool_entries
            .iter()
            .filter(|t| t.enabled)
            .filter(|t| self.skills.get(&t.name).is_some())
            .map(|t| {
                let schema = self
                    .skills
                    .get(&t.name)
                    .map(|s| s.schema().parameters.clone())
                    .unwrap_or_else(|| {
                        serde_json::json!({
                            "type": "object",
                            "properties": {},
                        })
                    });
                let description = t
                    .description
                    .clone()
                    .or_else(|| {
                        self.skills
                            .get(&t.name)
                            .map(|s| s.description().to_string())
                    })
                    .unwrap_or_default();
                ToolDefinition {
                    name: t.name.clone(),
                    description,
                    input_schema: schema,
                }
            })
            .collect()
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
                ChatContent::Blocks(_) => "[tool interaction]".to_string(),
            };
            transcript.push_str(&format!("{}: {}\n", msg.role, text));
        }

        let summary_prompt = vec![ChatMessage {
            role: "user".to_string(),
            content: format!(
                "Summarize the following conversation concisely, preserving key facts, decisions, and context:\n\n{}",
                transcript
            ),
        }];

        let options = CompletionOptions {
            model: model.map(|s| s.to_string()),
            max_tokens: Some(512),
            response_format: None,
        };

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

        // Read workspace state
        let state = self.workspace_state.read().await;
        let identity_display_name = state
            .identity
            .as_ref()
            .and_then(|doc| doc.frontmatter.display_name.clone());
        let soul_name = state
            .soul
            .as_ref()
            .and_then(|doc| doc.frontmatter.name.clone())
            .or(identity_display_name)
            .unwrap_or_else(|| "Orka".into());
        let soul_body = state
            .soul
            .as_ref()
            .map(|doc| doc.body.clone())
            .unwrap_or_default();
        let soul_model = state
            .soul
            .as_ref()
            .and_then(|doc| doc.frontmatter.model.clone());
        let soul_max_tokens = state
            .soul
            .as_ref()
            .and_then(|doc| doc.frontmatter.max_tokens);
        let max_entries = state
            .memory
            .as_ref()
            .and_then(|doc| doc.frontmatter.max_entries)
            .unwrap_or(50);
        let max_tokens_per_session = state
            .soul
            .as_ref()
            .and_then(|doc| doc.frontmatter.max_tokens_per_session);
        let summarization_model = state
            .memory
            .as_ref()
            .and_then(|doc| doc.frontmatter.summarization_model.clone());
        let summarization_threshold = state
            .memory
            .as_ref()
            .and_then(|doc| doc.frontmatter.summarization_threshold)
            .unwrap_or(max_entries * 2);
        let context_window = state
            .soul
            .as_ref()
            .and_then(|doc| doc.frontmatter.context_window_tokens)
            .unwrap_or(self.default_context_window);

        // Read tool definitions from TOOLS.md
        let tool_entries: Vec<ToolEntry> = state
            .tools
            .as_ref()
            .map(|doc| doc.frontmatter.tools.clone())
            .unwrap_or_default();
        let tools_body = state
            .tools
            .as_ref()
            .map(|doc| doc.body.clone())
            .unwrap_or_default();
        drop(state);

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
            }
        } else {
            text
        };

        // Check for direct skill invocation: !skill <name> key=val ...
        if let Some(rest) = text.strip_prefix("!skill ") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.is_empty() {
                let available = self.skills.list().join(", ");
                return Ok(vec![self.make_reply(
                    envelope,
                    format!("Usage: !skill <name> key=val ...\nAvailable skills: {available}"),
                )]);
            }

            let skill_name = parts[0];

            if self.skills.get(skill_name).is_none() {
                let available = self.skills.list().join(", ");
                return Ok(vec![self.make_reply(
                    envelope,
                    format!("Unknown skill: {skill_name}\nAvailable skills: {available}"),
                )]);
            }

            let mut args = HashMap::new();
            for part in &parts[1..] {
                if let Some((k, v)) = part.split_once('=') {
                    args.insert(k.to_string(), serde_json::Value::String(v.to_string()));
                }
            }

            let input = SkillInput {
                args,
                context: Some(orka_core::SkillContext {
                    secrets: self.secrets.clone(),
                }),
            };
            match self.skills.invoke(skill_name, input).await {
                Ok(output) => {
                    return Ok(vec![
                        self.make_reply(envelope, format!("[{skill_name}] {}", output.data))
                    ]);
                }
                Err(e) => {
                    return Ok(vec![self.make_reply(envelope, format!("Skill error: {e}"))]);
                }
            }
        }

        // If LLM is configured, run the agent loop
        if let Some(ref llm) = self.llm {
            // Build tool definitions from TOOLS.md + skill registry
            let tools = self.build_tool_definitions(&tool_entries);

            // Load conversation history from memory
            let memory_key = format!("conversation:{}", session.id);
            let history: Vec<ChatMessageExt> = match self.memory.recall(&memory_key).await {
                Ok(Some(entry)) => serde_json::from_value(entry.value).unwrap_or_default(),
                _ => Vec::new(),
            };

            // Build messages: history + current user message
            let mut messages = history;
            messages.push(ChatMessageExt {
                role: "user".to_string(),
                content: ChatContent::Text(text.clone()),
            });

            // System prompt from SOUL.md + TOOLS.md body (instructions)
            let mut system_prompt = if soul_body.is_empty() {
                format!("You are {soul_name}.")
            } else {
                format!("You are {soul_name}.\n\n{soul_body}")
            };
            if !tools_body.is_empty() {
                system_prompt.push_str("\n\n");
                system_prompt.push_str(&tools_body);
            }

            let options = CompletionOptions {
                model: soul_model.clone(),
                max_tokens: soul_max_tokens,
                response_format: None,
            };

            // Load accumulated token usage from memory
            let token_key = format!("tokens:{}", session.id);
            let mut session_tokens: u64 = match self.memory.recall(&token_key).await {
                Ok(Some(entry)) => entry.value.as_u64().unwrap_or(0),
                _ => 0,
            };

            // Agent loop: call LLM, execute tool calls, feed results back
            let mut final_text = String::new();
            let llm_model = soul_model.as_deref().unwrap_or("default").to_string();
            for iteration in 0..MAX_AGENT_ITERATIONS {
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
                let completion = match llm
                    .complete_with_tools(messages.clone(), &system_prompt, &tools, options.clone())
                    .await
                {
                    Ok(resp) => resp,
                    Err(e) => {
                        warn!(%e, "LLM call failed");
                        final_text = format!("Sorry, the LLM request failed: {e}\n\nPlease check the server logs for details.");
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
                    .emit(DomainEvent {
                        id: EventId::new(),
                        timestamp: chrono::Utc::now(),
                        kind: DomainEventKind::LlmCompleted {
                            message_id: envelope.id.clone(),
                            model: llm_model.clone(),
                            input_tokens: completion.usage.input_tokens,
                            output_tokens: completion.usage.output_tokens,
                            duration_ms: llm_duration_ms,
                        },
                        metadata: HashMap::new(),
                    })
                    .await;

                // Accumulate token usage
                let iteration_tokens =
                    (completion.usage.input_tokens + completion.usage.output_tokens) as u64;
                session_tokens += iteration_tokens;

                // Check token budget
                if let Some(budget) = max_tokens_per_session {
                    if session_tokens > budget {
                        warn!(session_tokens, budget, "token budget exceeded for session");
                        final_text = format!(
                            "I've reached the token budget for this session ({budget} tokens). Please start a new conversation."
                        );
                        break;
                    }
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
                    }
                }

                if tool_calls.is_empty() {
                    // No tool calls — final response
                    final_text = response_text;
                    messages.push(ChatMessageExt {
                        role: "assistant".to_string(),
                        content: ChatContent::Text(final_text.clone()),
                    });
                    break;
                }

                // LLM requested tool calls
                debug!(
                    iteration,
                    tool_count = tool_calls.len(),
                    "agent loop: executing tool calls"
                );

                // Record assistant text if any
                if !response_text.is_empty() {
                    messages.push(ChatMessageExt {
                        role: "assistant".to_string(),
                        content: ChatContent::Text(response_text),
                    });
                }

                // Execute tool calls in parallel
                let mut join_set = tokio::task::JoinSet::new();
                for call in &tool_calls {
                    info!(skill = %call.name, id = %call.id, "invoking skill via tool call");

                    self.event_sink
                        .emit(DomainEvent {
                            id: EventId::new(),
                            timestamp: chrono::Utc::now(),
                            kind: DomainEventKind::SkillInvoked {
                                skill_name: call.name.clone(),
                                message_id: envelope.id.clone(),
                            },
                            metadata: HashMap::new(),
                        })
                        .await;

                    let call_id = call.id.clone();
                    let call_name = call.name.clone();
                    let call_input = call.input.clone();
                    let skills = self.skills.clone();
                    let event_sink = self.event_sink.clone();
                    let message_id = envelope.id.clone();
                    let secrets = self.secrets.clone();

                    join_set.spawn(async move {
                        let args: HashMap<String, serde_json::Value> = call_input
                            .as_object()
                            .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                            .unwrap_or_default();

                        let start = std::time::Instant::now();
                        let skill_input = SkillInput {
                            args,
                            context: Some(orka_core::SkillContext { secrets }),
                        };
                        let result = skills.invoke(&call_name, skill_input).await;
                        let duration_ms = start.elapsed().as_millis() as u64;

                        let (content, is_error) = match &result {
                            Ok(output) => (output.data.to_string(), false),
                            Err(e) => (format!("Error: {e}"), true),
                        };

                        event_sink
                            .emit(DomainEvent {
                                id: EventId::new(),
                                timestamp: chrono::Utc::now(),
                                kind: DomainEventKind::SkillCompleted {
                                    skill_name: call_name,
                                    message_id,
                                    duration_ms,
                                    success: !is_error,
                                },
                                metadata: HashMap::new(),
                            })
                            .await;

                        (call_id, content, is_error)
                    });
                }

                // Collect results
                let mut results_map: HashMap<String, (String, bool)> = HashMap::new();
                while let Some(res) = join_set.join_next().await {
                    if let Ok((call_id, content, is_error)) = res {
                        results_map.insert(call_id, (content, is_error));
                    }
                }

                // Build result blocks in original order
                let mut result_blocks = Vec::new();
                for call in &tool_calls {
                    let (content, is_error) = results_map
                        .remove(&call.id)
                        .unwrap_or_else(|| ("Error: task failed".to_string(), true));
                    result_blocks.push(ContentBlockInput::ToolResult {
                        tool_use_id: call.id.clone(),
                        content,
                        is_error,
                    });
                }

                // Add tool results as a user message
                messages.push(ChatMessageExt {
                    role: "user".to_string(),
                    content: ChatContent::Blocks(result_blocks),
                });

                if iteration == MAX_AGENT_ITERATIONS - 1 {
                    warn!("agent loop reached max iterations ({MAX_AGENT_ITERATIONS})");
                    final_text =
                        "I reached the maximum number of reasoning steps. Here's what I have so far."
                            .to_string();
                }
            }

            if !final_text.is_empty() {
                // Save conversation history
                let history_to_save = if messages.len() > max_entries {
                    if let (Some(ref llm), true) =
                        (&self.llm, messages.len() > summarization_threshold)
                    {
                        // Summarize old messages
                        let split_point = messages.len() - max_entries;
                        let old_messages = &messages[..split_point];

                        let summary_text = Self::summarize_messages(
                            llm,
                            old_messages,
                            summarization_model.as_deref(),
                        )
                        .await;
                        let mut condensed = vec![ChatMessageExt {
                            role: "user".to_string(),
                            content: ChatContent::Text(format!(
                                "[Previous conversation summary: {}]",
                                summary_text
                            )),
                        }];
                        condensed.extend_from_slice(&messages[split_point..]);
                        condensed
                    } else {
                        messages[messages.len() - max_entries..].to_vec()
                    }
                } else {
                    messages
                };

                let entry = MemoryEntry {
                    key: memory_key.clone(),
                    value: serde_json::to_value(&history_to_save).unwrap_or_default(),
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    tags: vec!["conversation".to_string()],
                };
                let _ = self.memory.store(&memory_key, entry, None).await;

                // Persist accumulated token usage
                let token_entry = MemoryEntry {
                    key: token_key.clone(),
                    value: serde_json::json!(session_tokens),
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    tags: vec!["token_usage".to_string()],
                };
                let _ = self.memory.store(&token_key, token_entry, None).await;

                // Apply output guardrail
                let final_text = if let Some(ref guardrail) = self.guardrail {
                    match guardrail.check_output(&final_text, session).await? {
                        orka_core::traits::GuardrailDecision::Allow => final_text,
                        orka_core::traits::GuardrailDecision::Block(reason) => {
                            format!("I generated a response but it was filtered: {reason}")
                        }
                        orka_core::traits::GuardrailDecision::Modify(filtered) => filtered,
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
    use orka_core::testing::{InMemoryEventSink, InMemoryMemoryStore, InMemorySecretManager};
    use orka_core::SessionId;

    fn test_workspace_state(name: Option<&str>, body: &str) -> Arc<RwLock<WorkspaceState>> {
        use orka_workspace::config::SoulFrontmatter;
        use orka_workspace::parse::Document;

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
        Arc::new(RwLock::new(state))
    }

    fn test_workspace_state_with_tools(
        name: Option<&str>,
        body: &str,
        tool_entries: Vec<ToolEntry>,
    ) -> Arc<RwLock<WorkspaceState>> {
        use orka_workspace::config::{SoulFrontmatter, ToolsFrontmatter};
        use orka_workspace::parse::Document;

        let state = WorkspaceState {
            soul: Some(Document {
                frontmatter: SoulFrontmatter {
                    name: name.map(|s| s.to_string()),
                    ..Default::default()
                },
                body: body.to_string(),
            }),
            tools: Some(Document {
                frontmatter: ToolsFrontmatter {
                    tools: tool_entries,
                },
                body: String::new(),
            }),
            ..Default::default()
        };
        Arc::new(RwLock::new(state))
    }

    fn test_registry() -> Arc<SkillRegistry> {
        let mut reg = SkillRegistry::new();
        reg.register(Arc::new(orka_skills::EchoSkill));
        Arc::new(reg)
    }

    fn test_handler(state: Arc<RwLock<WorkspaceState>>) -> WorkspaceHandler {
        WorkspaceHandler::new(
            state,
            test_registry(),
            Arc::new(InMemoryMemoryStore::new()),
            Arc::new(InMemorySecretManager::new()),
            None,
            Arc::new(InMemoryEventSink::new()),
            128_000,
            None,
        )
    }

    #[tokio::test]
    async fn soul_name_in_reply() {
        let state = test_workspace_state(Some("TestBot"), "I am a test bot.");
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
            _ => panic!("expected text"),
        }
    }

    #[tokio::test]
    async fn skill_invocation() {
        let state = test_workspace_state(Some("Bot"), "");
        let handler = test_handler(state);

        let session = Session::new("custom", "user1");
        let envelope = Envelope::text("custom", SessionId::new(), "!skill echo greeting=world");

        let replies = handler.handle(&envelope, &session).await.unwrap();
        assert_eq!(replies.len(), 1);
        match &replies[0].payload {
            Payload::Text(t) => {
                assert!(t.contains("[echo]"));
                assert!(t.contains("greeting"));
                assert!(t.contains("world"));
            }
            _ => panic!("expected text"),
        }
    }

    #[tokio::test]
    async fn unknown_skill_error() {
        let state = test_workspace_state(Some("Bot"), "");
        let handler = test_handler(state);

        let session = Session::new("custom", "user1");
        let envelope = Envelope::text("custom", SessionId::new(), "!skill nonexistent");

        let replies = handler.handle(&envelope, &session).await.unwrap();
        assert_eq!(replies.len(), 1);
        match &replies[0].payload {
            Payload::Text(t) => {
                assert!(t.contains("Unknown skill: nonexistent"));
                assert!(t.contains("echo")); // available skills listed
            }
            _ => panic!("expected text"),
        }
    }

    #[tokio::test]
    async fn non_text_rejection() {
        let state = test_workspace_state(Some("Bot"), "");
        let handler = test_handler(state);

        let session = Session::new("custom", "user1");
        let mut envelope = Envelope::text("custom", SessionId::new(), "");
        envelope.payload = Payload::Command(orka_core::CommandPayload {
            name: "test".into(),
            args: HashMap::new(),
        });

        let replies = handler.handle(&envelope, &session).await.unwrap();
        assert_eq!(replies.len(), 1);
        match &replies[0].payload {
            Payload::Text(t) => assert!(t.contains("only process text")),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn build_tool_definitions_filters_enabled_and_registered() {
        let tool_entries = vec![
            ToolEntry {
                name: "echo".to_string(),
                enabled: true,
                description: Some("Custom echo desc".to_string()),
                config: HashMap::new(),
            },
            ToolEntry {
                name: "nonexistent".to_string(),
                enabled: true,
                description: None,
                config: HashMap::new(),
            },
            ToolEntry {
                name: "echo".to_string(),
                enabled: false,
                description: None,
                config: HashMap::new(),
            },
        ];

        let state = test_workspace_state_with_tools(Some("Bot"), "", tool_entries.clone());
        let handler = test_handler(state);

        let defs = handler.build_tool_definitions(&tool_entries);
        // Only the first entry passes: enabled=true and "echo" exists in registry
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "echo");
        assert_eq!(defs[0].description, "Custom echo desc");
    }
}
