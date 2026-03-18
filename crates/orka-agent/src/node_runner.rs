//! Single-agent LLM tool loop, extracted from `workspace_handler.rs`.

use std::collections::HashMap;

use orka_core::{DomainEvent, DomainEventKind, SkillInput};
use orka_llm::client::{
    ChatContent, ChatMessageExt, CompletionOptions, ContentBlock, ContentBlockInput, LlmToolStream,
    StreamEvent, ToolCall, ToolDefinition, Usage,
};
use orka_llm::context::{available_history_budget, truncate_history};
use tracing::{Instrument, debug, info, info_span, warn};

use crate::agent::Agent;
use crate::context::ExecutionContext;
use crate::executor::ExecutorDeps;
use crate::graph::AgentGraph;
use crate::handoff::{Handoff, HandoffMode};
use crate::tools::build_handoff_tools;

/// The result of running a single agent node.
#[derive(Debug)]
pub struct AgentNodeResult {
    /// The agent's final text response, if it produced one.
    pub response: Option<String>,
    /// A handoff request, if the agent decided to transfer/delegate.
    pub handoff: Option<Handoff>,
    /// Number of LLM iterations consumed.
    pub iterations: usize,
}

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

/// Consume a streaming LLM response, rebuilding a `CompletionResponse`.
pub async fn consume_stream(
    mut stream: LlmToolStream,
    session_id: &orka_core::SessionId,
    stream_registry: &orka_core::StreamRegistry,
    channel: &str,
    reply_to: Option<&orka_core::MessageId>,
) -> orka_core::Result<orka_llm::client::CompletionResponse> {
    use futures_util::StreamExt;
    use orka_core::stream::{StreamChunk, StreamChunkKind};

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
            other => {
                debug!(?other, "unhandled stream event");
            }
        }
    }

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

    Ok(orka_llm::client::CompletionResponse::new(
        blocks,
        usage,
        stop_reason,
    ))
}

/// Execute a single agent's LLM tool loop.
///
/// This is the core of the agent system, adapted to operate on
/// `(Agent, ExecutionContext, ExecutorDeps)`.
pub async fn run_agent_node(
    agent: &Agent,
    ctx: &ExecutionContext,
    deps: &ExecutorDeps,
    graph: &AgentGraph,
) -> orka_core::Result<AgentNodeResult> {
    let llm = match &deps.llm {
        Some(l) => l.clone(),
        None => {
            return Ok(AgentNodeResult {
                response: Some("No LLM provider configured.".into()),
                handoff: None,
                iterations: 0,
            });
        }
    };

    // Build tool list filtered by ToolScope
    let mut tools: Vec<ToolDefinition> = deps
        .skills
        .list()
        .iter()
        .filter(|name| agent.tools.allows(name))
        .filter_map(|name| deps.skills.get(name))
        .map(|skill| {
            ToolDefinition::new(
                skill.name(),
                skill.description(),
                skill.schema().parameters.clone(),
            )
        })
        .collect();

    // Inject handoff tools
    let handoff_tools = build_handoff_tools(agent, graph);
    tools.extend(handoff_tools);

    // Build system prompt
    let system_prompt = {
        let mut sp = agent.system_prompt.build(&agent.display_name);

        // Inject learned principles if experience is available
        if let Some(ref exp) = deps.experience {
            let trigger_text = match &ctx.trigger.payload {
                orka_core::Payload::Text(t) => t.clone(),
                _ => String::new(),
            };
            match exp
                .retrieve_principles(&trigger_text, agent.id.0.as_ref())
                .await
            {
                Ok(principles) if !principles.is_empty() => {
                    let section =
                        orka_experience::ExperienceService::format_principles_section(&principles);
                    if !section.is_empty() {
                        sp.push_str(&section);
                        deps.event_sink
                            .emit(DomainEvent::new(DomainEventKind::PrinciplesInjected {
                                session_id: ctx.session_id,
                                count: principles.len(),
                            }))
                            .await;
                    }
                }
                Err(e) => {
                    warn!(%e, "failed to retrieve principles");
                }
                _ => {}
            }
        }

        sp
    };

    let mut options = CompletionOptions::default();
    options.model = agent.llm_config.model.clone();
    options.max_tokens = agent.llm_config.max_tokens;

    let context_window = agent.llm_config.context_window.unwrap_or(200_000);
    let output_budget = agent.llm_config.max_tokens.unwrap_or(4096);

    let max_tool_retries: u32 = 2;
    let mut tool_error_counts: HashMap<String, u32> = HashMap::new();
    let max_result_chars: usize = 50_000;
    let skill_timeout = std::time::Duration::from_secs(120);

    let envelope = &ctx.trigger;
    let message_id = envelope.id;

    let mut messages = ctx.messages().await;
    let mut iterations = 0usize;
    let mut final_response: Option<String> = None;
    let mut handoff: Option<Handoff> = None;

    for iteration in 0..agent.max_iterations {
        iterations = iteration + 1;
        let iteration_start = std::time::Instant::now();

        // Truncate history to fit context window
        let budget =
            available_history_budget(context_window, output_budget, &system_prompt, &tools);
        let (truncated, dropped) = truncate_history(messages, budget);
        messages = truncated;
        if dropped > 0 {
            warn!(
                dropped,
                remaining = messages.len(),
                "truncated history to fit context window"
            );
        }

        let llm_span = info_span!(
            "llm.call",
            agent_id = %agent.id,
            iteration,
            model = %agent.llm_config.model.as_deref().unwrap_or("default"),
        );

        let llm_start = std::time::Instant::now();

        let stream = match llm
            .complete_stream_with_tools(&messages, &system_prompt, &tools, options.clone())
            .instrument(llm_span.clone())
            .await
        {
            Ok(s) => s,
            Err(e) => {
                warn!(%e, "LLM stream init failed");
                final_response = Some(format!("LLM request failed: {e}"));
                break;
            }
        };

        let completion = match consume_stream(
            stream,
            &ctx.session_id,
            &deps.stream_registry,
            &envelope.channel,
            Some(&envelope.id),
        )
        .instrument(llm_span)
        .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(%e, "LLM stream failed");
                final_response = Some(format!("LLM request failed: {e}"));
                break;
            }
        };

        let llm_duration_ms = llm_start.elapsed().as_millis() as u64;
        let iteration_tokens =
            (completion.usage.input_tokens + completion.usage.output_tokens) as u64;
        ctx.add_tokens(iteration_tokens);

        deps.event_sink
            .emit(DomainEvent::new(DomainEventKind::LlmCompleted {
                message_id,
                model: agent
                    .llm_config
                    .model
                    .clone()
                    .unwrap_or_else(|| "default".into()),
                input_tokens: completion.usage.input_tokens,
                output_tokens: completion.usage.output_tokens,
                duration_ms: llm_duration_ms,
                estimated_cost_usd: None,
            }))
            .await;

        // Parse response
        let mut response_text = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        for block in &completion.blocks {
            match block {
                ContentBlock::Text(t) => response_text.push_str(t),
                ContentBlock::ToolUse(call) => tool_calls.push(call.clone()),
                other => {
                    debug!(?other, "unhandled content block");
                }
            }
        }

        if !response_text.is_empty() {
            deps.event_sink
                .emit(DomainEvent::new(DomainEventKind::AgentReasoning {
                    message_id,
                    iteration,
                    reasoning_text: response_text.clone(),
                }))
                .await;
        }

        // Check for handoff tool calls first
        let mut handoff_call: Option<ToolCall> = None;
        let mut regular_calls: Vec<ToolCall> = Vec::new();

        for call in tool_calls {
            if call.name == "transfer_to_agent" || call.name == "delegate_to_agent" {
                handoff_call = Some(call);
            } else {
                regular_calls.push(call);
            }
        }

        if regular_calls.is_empty() && handoff_call.is_none() {
            // No tool calls — final response
            messages.push(ChatMessageExt::assistant(response_text.clone()));
            final_response = Some(response_text);

            deps.event_sink
                .emit(DomainEvent::new(DomainEventKind::AgentIteration {
                    message_id,
                    iteration,
                    tool_count: 0,
                    tokens_used: iteration_tokens,
                    elapsed_ms: iteration_start.elapsed().as_millis() as u64,
                }))
                .await;
            break;
        }

        if let Some(ref hc) = handoff_call {
            // Parse handoff parameters
            let target_id_str = hc
                .input
                .get("agent_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let reason = hc
                .input
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let context_map: HashMap<String, serde_json::Value> = hc
                .input
                .get("context")
                .and_then(|v| v.as_object())
                .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default();

            let mode = if hc.name == "transfer_to_agent" {
                HandoffMode::Transfer
            } else {
                HandoffMode::Delegate
            };

            info!(
                from = %agent.id,
                to = %target_id_str,
                ?mode,
                "agent handoff"
            );

            handoff = Some(Handoff {
                from: agent.id.clone(),
                to: crate::agent::AgentId::from(target_id_str),
                reason,
                context_transfer: context_map,
                mode,
            });

            // Push the assistant message with the handoff tool call
            let mut blocks = Vec::new();
            if !response_text.is_empty() {
                blocks.push(ContentBlockInput::Text {
                    text: response_text,
                });
            }
            blocks.push(ContentBlockInput::ToolUse {
                id: hc.id.clone(),
                name: hc.name.clone(),
                input: hc.input.clone(),
            });
            if !blocks.is_empty() {
                messages.push(ChatMessageExt::new(
                    orka_llm::client::Role::Assistant,
                    ChatContent::Blocks(blocks),
                ));
            }
            break;
        }

        // Execute regular tool calls in parallel
        {
            let mut blocks = Vec::new();
            if !response_text.is_empty() {
                blocks.push(ContentBlockInput::Text {
                    text: response_text,
                });
            }
            for call in &regular_calls {
                blocks.push(ContentBlockInput::ToolUse {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    input: call.input.clone(),
                });
            }
            messages.push(ChatMessageExt::new(
                orka_llm::client::Role::Assistant,
                ChatContent::Blocks(blocks),
            ));
        }

        let mut join_set = tokio::task::JoinSet::new();

        for call in &regular_calls {
            let call_id = call.id.clone();
            let call_name = call.name.clone();
            let call_input = call.input.clone();
            let skills = deps.skills.clone();
            let event_sink = deps.event_sink.clone();
            let secrets = deps.secrets.clone();

            deps.event_sink
                .emit(DomainEvent::new(DomainEventKind::SkillInvoked {
                    skill_name: call.name.clone(),
                    message_id,
                }))
                .await;

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

                (call_id, content, is_error)
            });
        }

        let mut results_map: HashMap<String, (String, bool)> = HashMap::new();
        while let Some(res) = join_set.join_next().await {
            if let Ok((call_id, content, is_error)) = res {
                results_map.insert(call_id, (content, is_error));
            }
        }

        // Build result blocks in original order
        let mut result_blocks: Vec<ContentBlockInput> = regular_calls
            .iter()
            .map(|call| {
                let (content, is_error) = results_map
                    .remove(&call.id)
                    .unwrap_or_else(|| ("Error: task failed".to_string(), true));
                ContentBlockInput::ToolResult {
                    tool_use_id: call.id.clone(),
                    content,
                    is_error,
                }
            })
            .collect();

        // Track errors and inject self-correction hints
        for (block, call) in result_blocks.iter().zip(regular_calls.iter()) {
            if let ContentBlockInput::ToolResult { is_error, .. } = block {
                if *is_error {
                    let count = tool_error_counts.entry(call.name.clone()).or_insert(0);
                    *count += 1;
                } else {
                    tool_error_counts.remove(&call.name);
                }
            }
        }

        let mut hint_text: Option<String> = None;
        for (tool_name, count) in &tool_error_counts {
            if *count >= max_tool_retries {
                hint_text = Some(format!(
                    "Tool '{tool_name}' has failed {count} consecutive times. Consider an alternative approach."
                ));
                break;
            }
        }
        if let Some(hint) = hint_text {
            result_blocks.push(ContentBlockInput::Text { text: hint });
        }

        messages.push(ChatMessageExt::new(
            orka_llm::client::Role::User,
            ChatContent::Blocks(result_blocks),
        ));

        deps.event_sink
            .emit(DomainEvent::new(DomainEventKind::AgentIteration {
                message_id,
                iteration,
                tool_count: regular_calls.len(),
                tokens_used: iteration_tokens,
                elapsed_ms: iteration_start.elapsed().as_millis() as u64,
            }))
            .await;

        if iteration == agent.max_iterations - 1 {
            warn!(
                max_iterations = agent.max_iterations,
                "agent reached max iterations"
            );
            final_response = Some("I reached the maximum number of reasoning steps.".to_string());
        }
    }

    ctx.set_messages(messages).await;

    Ok(AgentNodeResult {
        response: final_response,
        handoff,
        iterations,
    })
}
