//! Single-agent LLM tool loop, extracted from `workspace_handler.rs`.

use std::{collections::HashMap, sync::Arc};

use orka_core::{
    DomainEvent, DomainEventKind, SkillInput, SkillOutput,
    traits::{EventSink, GuardrailDecision, SecretManager},
    truncate_tool_result,
    types::MediaPayload,
};
use orka_llm::{
    client::{
        ChatContent, ChatMessage, CompletionOptions, CompletionResponse, ContentBlock,
        ContentBlockInput, ToolCall, ToolDefinition,
    },
    consume_stream,
    context::{
        TokenizerHint, available_history_budget_with_hint, estimate_message_tokens_with_hint,
        sanitize_tool_result_history, truncate_history_with_hint,
    },
    infer_provider,
};
use orka_prompts::pipeline::PipelineConfig;
use tracing::{Instrument, debug, info, info_span, warn};

use crate::{
    agent::{Agent, HistoryStrategy},
    context::{ExecutionContext, SlotKey},
    executor::ExecutorDeps,
    graph::AgentGraph,
    handoff::{Handoff, HandoffMode},
    planner::{PLAN_SLOT, Plan, PlanStep, PlanningMode, StepStatus, planning_tools},
    tools::build_handoff_tools,
};

/// The result of running a single agent node.
#[derive(Debug)]
pub(crate) struct AgentNodeResult {
    /// The agent's final text response, if it produced one.
    pub response: Option<String>,
    /// A handoff request, if the agent decided to transfer/delegate.
    pub handoff: Option<Handoff>,
    /// Number of LLM iterations consumed.
    pub iterations: usize,
    /// Set when the agent was interrupted before executing a tool that requires
    /// human approval. The executor saves an `Interrupted` checkpoint and
    /// stops.
    pub interrupted: Option<orka_checkpoint::InterruptReason>,
    /// Media attachments produced by skills during this node's execution.
    /// Forwarded by the executor as separate outbound media messages.
    pub attachments: Vec<MediaPayload>,
    /// Why the agent stopped executing.
    pub stop_reason: orka_core::stream::AgentStopReason,
}

/// Parse a handoff from a tool call issued by the LLM.
///
/// `call.name` must be `"transfer_to_agent"` or `"delegate_to_agent"`.
pub(crate) fn parse_handoff(
    from: &crate::agent::AgentId,
    call: &orka_llm::client::ToolCall,
) -> Handoff {
    let to = call
        .input
        .get("agent_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let reason = call
        .input
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let context_transfer: HashMap<String, serde_json::Value> = call
        .input
        .get("context")
        .and_then(|v| v.as_object())
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    let mode = if call.name == "transfer_to_agent" {
        HandoffMode::Transfer
    } else {
        HandoffMode::Delegate
    };
    Handoff {
        from: from.clone(),
        to: crate::agent::AgentId::from(to),
        reason,
        context_transfer,
        mode,
    }
}

/// Build a self-correction hint when a tool has failed too many times
/// consecutively.  Returns `None` if no tool has reached the threshold.
pub(crate) fn build_tool_error_hint(
    counts: &HashMap<String, u32>,
    max_retries: u32,
) -> Option<String> {
    counts.iter().find_map(|(name, &count)| {
        (count >= max_retries).then(|| {
            format!(
                "Tool '{name}' has failed {count} consecutive times. \
                 Consider an alternative approach."
            )
        })
    })
}

// ---------------------------------------------------------------------------
// Outcome enum for the main loop
// ---------------------------------------------------------------------------

enum IterationOutcome {
    Continue,
    Done,
    Interrupted(orka_checkpoint::InterruptReason),
}

// ---------------------------------------------------------------------------
// Context structs to avoid too-many-parameters
// ---------------------------------------------------------------------------

struct TriggerContext {
    trigger_text: String,
    workspace_name: String,
    available_workspaces: Vec<String>,
    cwd: Option<String>,
    message_id: orka_core::types::MessageId,
}

// ---------------------------------------------------------------------------
// Runner struct
// ---------------------------------------------------------------------------

struct AgentNodeRunner<'a> {
    agent: &'a Agent,
    ctx: &'a ExecutionContext,
    deps: &'a ExecutorDeps,
    llm: Arc<dyn orka_llm::LlmClient>,
    message_id: orka_core::types::MessageId,
    progressive: bool,
    enabled_categories: std::collections::HashSet<String>,
    handoff_tools: Vec<ToolDefinition>,
    plan_tools: Vec<ToolDefinition>,
    tools: Vec<ToolDefinition>,
    system_prompt: String,
    options: CompletionOptions,
    context_window: u32,
    output_budget: u32,
    messages: Vec<ChatMessage>,
    max_result_chars: usize,
    skill_timeout: std::time::Duration,
    tool_error_counts: HashMap<String, u32>,
    max_tool_retries: u32,
    iterations: usize,
    tool_turns: usize,
    final_response: Option<String>,
    handoff: Option<Handoff>,
    stop_reason: orka_core::stream::AgentStopReason,
    collected_attachments: Vec<MediaPayload>,
    worktree_cwd: Option<String>,
}

impl<'a> AgentNodeRunner<'a> {
    async fn new(
        agent: &'a Agent,
        ctx: &'a ExecutionContext,
        deps: &'a ExecutorDeps,
        graph: &AgentGraph,
        llm: Arc<dyn orka_llm::LlmClient>,
    ) -> orka_core::Result<Self> {
        let progressive = agent.progressive_disclosure;
        let enabled_categories = std::collections::HashSet::new();

        let trigger_context = build_trigger_context(agent, ctx);
        let TriggerContext {
            trigger_text,
            workspace_name,
            available_workspaces,
            cwd,
            message_id,
        } = trigger_context;

        let handoff_tools = build_handoff_tools(agent, graph);
        let plan_tools = if agent.planning_mode == PlanningMode::Adaptive {
            planning_tools()
        } else {
            vec![]
        };

        let initial_skill_tools = build_skill_tools(deps, agent, progressive, &enabled_categories);
        let tools = assemble_tools(
            progressive,
            initial_skill_tools,
            handoff_tools.clone(),
            plan_tools.clone(),
        );

        // History summarization (HistoryStrategy::Summarize)
        maybe_summarize_history(ctx, deps, agent).await;

        let system_prompt = build_agent_system_prompt(
            agent,
            ctx,
            deps,
            &trigger_text,
            &workspace_name,
            &available_workspaces,
            cwd.as_deref(),
        )
        .await;

        let mut options = CompletionOptions::default();
        options.model = agent.llm_config.model.clone();
        options.max_tokens = agent.llm_config.max_tokens;
        options.temperature = agent.llm_config.temperature;
        options.thinking = agent.llm_config.thinking.clone();

        let context_window = agent.llm_config.context_window.unwrap_or(200_000);
        let output_budget = agent.llm_config.max_tokens.unwrap_or(4096);
        let max_tool_retries: u32 = 2;
        let max_result_chars = agent.tool_result_max_chars;
        let skill_timeout = std::time::Duration::from_secs(agent.skill_timeout_secs);

        let mut messages = load_history(agent, ctx).await;

        // Input guardrail: modify message if needed (block is handled in
        // run_agent_node).
        messages = apply_input_guardrail_modify(deps, ctx, &trigger_text, messages).await;

        // PlanningMode::Always — eager plan generation
        let system_prompt =
            maybe_generate_plan(agent, ctx, deps, &trigger_text, system_prompt).await;

        Ok(Self {
            agent,
            ctx,
            deps,
            llm,
            message_id,
            progressive,
            enabled_categories,
            handoff_tools,
            plan_tools,
            tools,
            system_prompt,
            options,
            context_window,
            output_budget,
            messages,
            max_result_chars,
            skill_timeout,
            tool_error_counts: HashMap::new(),
            max_tool_retries,
            iterations: 0,
            tool_turns: 0,
            final_response: None,
            handoff: None,
            stop_reason: orka_core::stream::AgentStopReason::Complete,
            collected_attachments: Vec::new(),
            worktree_cwd: None,
        })
    }

    #[allow(clippy::too_many_lines)]
    async fn run(mut self) -> orka_core::Result<AgentNodeResult> {
        let run_start = std::time::Instant::now();
        let llm_call_timeout = std::time::Duration::from_secs(self.agent.llm_call_timeout_secs);

        loop {
            // Fix 4: wall-clock run timeout
            if let Some(max_run_secs) = self.agent.max_run_secs {
                let elapsed = run_start.elapsed().as_secs();
                if elapsed >= max_run_secs {
                    warn!(
                        max_run_secs,
                        elapsed_secs = elapsed,
                        "agent run exceeded wall-clock time limit"
                    );
                    self.final_response = Some(format!(
                        "I'm sorry, this request exceeded the maximum allowed time \
                         ({max_run_secs}s) and was stopped automatically."
                    ));
                    self.stop_reason = orka_core::stream::AgentStopReason::MaxTurns;
                    break;
                }
            }

            let iteration = self.iterations;
            self.iterations += 1;
            let iteration_start = std::time::Instant::now();

            let (sanitized, removed) = sanitize_tool_result_history(self.messages);
            self.messages = sanitized;
            if removed > 0 {
                warn!(
                    agent = %self.agent.id,
                    iteration,
                    removed_tool_results = removed,
                    "dropped orphaned tool_result blocks before llm call"
                );
            }

            self.rebuild_tools();
            self.manage_context_window(iteration).await;

            // Fix 1: per-call LLM timeout
            let completion =
                match tokio::time::timeout(llm_call_timeout, self.call_llm(iteration)).await {
                    Ok(Ok(c)) => c,
                    Ok(Err(_)) => break, // error already handled in call_llm (sets final_response)
                    Err(_elapsed) => {
                        warn!(
                            timeout_secs = self.agent.llm_call_timeout_secs,
                            iteration, "LLM call timed out"
                        );
                        self.final_response = Some(format!(
                            "I'm sorry, the LLM did not respond within the allowed time \
                         ({}s) and the request was interrupted.",
                            self.agent.llm_call_timeout_secs
                        ));
                        self.stop_reason = orka_core::stream::AgentStopReason::Error;
                        break;
                    }
                };

            let iteration_tokens =
                u64::from(completion.usage.input_tokens + completion.usage.output_tokens);
            let (response_text, tool_calls) =
                parse_completion_blocks(self.agent, self.deps, &completion, self.message_id).await;

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
                self.messages
                    .push(ChatMessage::assistant(response_text.clone()));
                self.final_response = Some(response_text);
                emit_iteration_event(
                    self.deps,
                    self.message_id,
                    iteration,
                    0,
                    iteration_tokens,
                    &iteration_start,
                )
                .await;
                break;
            }

            if let Some(ref hc) = handoff_call {
                let h = parse_handoff(&self.agent.id, hc);
                info!(from = %self.agent.id, to = %h.to, mode = ?h.mode, "agent handoff");
                self.handoff = Some(h);
                push_handoff_assistant_message(&mut self.messages, &response_text, hc);
                break;
            }

            push_tool_call_assistant_message(&mut self.messages, &response_text, &regular_calls);

            match self
                .dispatch_tool_calls(regular_calls, response_text, iteration, iteration_start)
                .await?
            {
                IterationOutcome::Done => break,
                IterationOutcome::Interrupted(reason) => {
                    return Ok(AgentNodeResult {
                        response: None,
                        handoff: None,
                        iterations: self.iterations,
                        interrupted: Some(reason),
                        attachments: Vec::new(),
                        stop_reason: orka_core::stream::AgentStopReason::Interrupted,
                    });
                }
                IterationOutcome::Continue => {}
            }
        }

        self.ctx.set_messages(self.messages).await;

        let final_response =
            apply_output_guardrail(self.agent, self.ctx, self.deps, self.final_response).await;

        Ok(AgentNodeResult {
            response: final_response,
            handoff: self.handoff,
            iterations: self.iterations,
            interrupted: None,
            attachments: self.collected_attachments,
            stop_reason: self.stop_reason,
        })
    }

    fn rebuild_tools(&mut self) {
        if self.progressive {
            self.tools.clear();
            self.tools.extend(synthetic_tools());
            self.tools.extend(build_skill_tools(
                self.deps,
                self.agent,
                self.progressive,
                &self.enabled_categories,
            ));
            self.tools.extend(self.handoff_tools.clone());
            self.tools.extend(self.plan_tools.clone());
        }
    }

    async fn manage_context_window(&mut self, iteration: usize) {
        let _ = iteration;
        let hint = TokenizerHint::from_model(self.agent.llm_config.model.as_deref());
        let envelope = &self.ctx.trigger;

        if let HistoryStrategy::RollingWindow { recent_turns } = self.agent.history_strategy {
            let turns = orka_llm::context::group_into_turns(&self.messages);
            if turns.len() > recent_turns {
                let cutoff = turns.len() - recent_turns;
                let drop_end = turns[cutoff - 1].end;
                let to_drop = self.messages[..drop_end].to_vec();
                if let Some(ref llm_client) = self.deps.llm
                    && let Ok(summary) = summarize_messages(llm_client.as_ref(), &to_drop).await
                {
                    self.ctx.set_conversation_summary(summary).await;
                }
                let dropped = drop_end;
                self.messages = self.messages[drop_end..].to_vec();
                warn!(
                    dropped,
                    remaining = self.messages.len(),
                    recent_turns,
                    "rolling window: trimmed history"
                );
                let history_tokens: u32 = self
                    .messages
                    .iter()
                    .map(|m| estimate_message_tokens_with_hint(m, hint))
                    .sum();
                self.deps
                    .stream_registry
                    .send(orka_core::stream::StreamChunk::new(
                        self.ctx.session_id,
                        envelope.channel.clone(),
                        Some(envelope.id),
                        orka_core::stream::StreamChunkKind::ContextInfo {
                            history_tokens,
                            context_window: self.context_window,
                            messages_truncated: dropped as u32,
                            summary_generated: true,
                        },
                    ));
            }
        } else {
            let budget = available_history_budget_with_hint(
                self.context_window,
                self.output_budget,
                &self.system_prompt,
                &self.tools,
                hint,
            );
            let (truncated, dropped) =
                truncate_history_with_hint(self.messages.clone(), budget, hint, true);
            self.messages = truncated;
            if dropped > 0 {
                warn!(
                    dropped,
                    remaining = self.messages.len(),
                    "truncated history to fit context window"
                );
                let history_tokens: u32 = self
                    .messages
                    .iter()
                    .map(|m| estimate_message_tokens_with_hint(m, hint))
                    .sum();
                self.deps
                    .stream_registry
                    .send(orka_core::stream::StreamChunk::new(
                        self.ctx.session_id,
                        envelope.channel.clone(),
                        Some(envelope.id),
                        orka_core::stream::StreamChunkKind::ContextInfo {
                            history_tokens,
                            context_window: self.context_window,
                            messages_truncated: dropped as u32,
                            summary_generated: false,
                        },
                    ));
            }
        }
    }

    async fn call_llm(
        &mut self,
        iteration: usize,
    ) -> orka_core::Result<orka_llm::CompletionResponse> {
        let envelope = &self.ctx.trigger;
        let llm_span = info_span!(
            "llm.call",
            agent_id = %self.agent.id,
            iteration,
            model = %self.agent.llm_config.model.as_deref().unwrap_or("default"),
        );
        let llm_start = std::time::Instant::now();

        let stream = match self
            .llm
            .complete_stream_with_tools(
                &self.messages,
                &self.system_prompt,
                &self.tools,
                &self.options,
            )
            .instrument(llm_span.clone())
            .await
        {
            Ok(s) => s,
            Err(e) => {
                warn!(%e, "LLM stream init failed");
                self.final_response = Some(format!("LLM request failed: {e}"));
                self.stop_reason = orka_core::stream::AgentStopReason::Error;
                return Err(orka_core::Error::llm(e, "llm stream init failed"));
            }
        };

        let completion = match consume_stream(
            stream,
            &self.ctx.session_id,
            &self.deps.stream_registry,
            &envelope.channel,
            Some(&envelope.id),
        )
        .instrument(llm_span)
        .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(%e, "LLM stream failed");
                self.final_response = Some(format!("LLM request failed: {e}"));
                self.stop_reason = orka_core::stream::AgentStopReason::Error;
                return Err(orka_core::Error::llm(e, "llm stream failed"));
            }
        };

        if completion.stop_reason == Some(orka_llm::client::StopReason::MaxTokens) {
            warn!("LLM response truncated (max_tokens reached)");
            self.stop_reason = orka_core::stream::AgentStopReason::MaxTokens;
        }

        let llm_duration_ms = llm_start.elapsed().as_millis() as u64;
        let iteration_tokens =
            u64::from(completion.usage.input_tokens + completion.usage.output_tokens);
        self.ctx.add_tokens(iteration_tokens);

        let llm_model = emit_llm_completed_event(
            self.deps,
            self.agent,
            self.message_id,
            &completion,
            llm_duration_ms,
        )
        .await;

        self.deps
            .stream_registry
            .send(orka_core::stream::StreamChunk::new(
                self.ctx.session_id,
                envelope.channel.clone(),
                Some(envelope.id),
                orka_core::stream::StreamChunkKind::Usage {
                    input_tokens: completion.usage.input_tokens,
                    output_tokens: completion.usage.output_tokens,
                    cache_read_tokens: (completion.usage.cache_read_input_tokens > 0)
                        .then_some(completion.usage.cache_read_input_tokens),
                    cache_creation_tokens: (completion.usage.cache_creation_input_tokens > 0)
                        .then_some(completion.usage.cache_creation_input_tokens),
                    reasoning_tokens: (completion.usage.reasoning_tokens > 0)
                        .then_some(completion.usage.reasoning_tokens),
                    model: llm_model,
                    cost_usd: None,
                },
            ));

        Ok(completion)
    }

    async fn dispatch_tool_calls(
        &mut self,
        regular_calls: Vec<ToolCall>,
        response_text: String,
        iteration: usize,
        iteration_start: std::time::Instant,
    ) -> orka_core::Result<IterationOutcome> {
        let iteration_tokens = 0u64; // already counted in call_llm

        let mut results_map: HashMap<String, (String, bool)> = HashMap::new();
        let mut skill_calls: Vec<ToolCall> = Vec::new();

        for call in &regular_calls {
            if let Some(result) = handle_plan_tool(self.agent, self.ctx, call).await {
                results_map.insert(call.id.clone(), result);
            } else if let Some(result) =
                handle_progressive_tool(call, self.progressive, &mut self.enabled_categories)
            {
                results_map.insert(call.id.clone(), result);
            } else {
                skill_calls.push(call.clone());
            }
        }

        // HITL: interrupt before running any tool requiring approval
        if let Some(call) = skill_calls
            .iter()
            .find(|c| self.agent.interrupt_before_tools.contains(&c.name))
        {
            let reason = orka_checkpoint::InterruptReason::HumanApproval {
                tool_name: call.name.clone(),
                tool_input: call.input.clone(),
                agent_id: self.agent.id.to_string(),
            };
            return Ok(IterationOutcome::Interrupted(reason));
        }

        // Apply per-tool guardrail, then run in parallel
        let (checked_calls, guardrail_blocks) =
            apply_tool_guardrails(self.agent, self.ctx, self.deps, skill_calls).await;
        results_map.extend(guardrail_blocks);

        let batch_results = self.execute_skill_batch_parallel(checked_calls).await;
        for (call_id, content, is_error, attachments) in batch_results {
            results_map.insert(call_id, (content, is_error));
            self.collected_attachments.extend(attachments);
        }

        update_worktree_cwd(&regular_calls, &results_map, &mut self.worktree_cwd);

        let mut result_blocks = build_result_blocks(&regular_calls, &mut results_map);

        for (block, call) in result_blocks.iter().zip(regular_calls.iter()) {
            if let ContentBlockInput::ToolResult { is_error, .. } = block {
                if *is_error {
                    let count = self.tool_error_counts.entry(call.name.clone()).or_insert(0);
                    *count += 1;
                } else {
                    self.tool_error_counts.remove(&call.name);
                }
            }
        }

        if let Some(hint) = build_tool_error_hint(&self.tool_error_counts, self.max_tool_retries) {
            result_blocks.push(ContentBlockInput::Text { text: hint });
        }

        self.messages.push(ChatMessage::new(
            orka_llm::client::Role::User,
            ChatContent::Blocks(result_blocks),
        ));

        let _ = response_text;
        emit_iteration_event(
            self.deps,
            self.message_id,
            iteration,
            regular_calls.len(),
            iteration_tokens,
            &iteration_start,
        )
        .await;

        self.tool_turns += 1;
        if self.tool_turns >= self.agent.max_turns {
            warn!(
                max_turns = self.agent.max_turns,
                "agent reached max tool turns"
            );
            self.final_response = Some(format!(
                "I've reached the maximum number of steps ({}) for this request. \
                 Please try rephrasing or breaking the task into smaller parts.",
                self.agent.max_turns
            ));
            self.stop_reason = orka_core::stream::AgentStopReason::MaxTurns;
            return Ok(IterationOutcome::Done);
        }

        Ok(IterationOutcome::Continue)
    }

    async fn execute_skill_batch_parallel(
        &mut self,
        skill_calls: Vec<ToolCall>,
    ) -> Vec<(String, String, bool, Vec<MediaPayload>)> {
        let mut join_set = tokio::task::JoinSet::new();

        for call in &skill_calls {
            let params = self.prepare_skill_task(call).await;
            join_set.spawn(invoke_skill_task(params));
        }

        let mut results = Vec::new();
        while let Some(res) = join_set.join_next().await {
            if let Ok(tuple) = res {
                results.push(tuple);
            }
        }
        results
    }

    async fn prepare_skill_task(&mut self, call: &ToolCall) -> SkillTaskParams {
        let message_id = self.message_id;
        self.deps
            .event_sink
            .emit(DomainEvent::new(DomainEventKind::SkillInvoked {
                skill_name: call.name.clone(),
                message_id,
                input_args: match &call.input {
                    serde_json::Value::Object(map) => {
                        map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
                    }
                    _ => HashMap::new(),
                },
                caller_id: None,
            }))
            .await;

        let user_cwd = self
            .ctx
            .trigger
            .metadata
            .get("workspace:cwd")
            .and_then(|v| v.as_str())
            .map(String::from);

        let progress_tx =
            maybe_spawn_progress_bridge(&call.name, &self.ctx.trigger.channel, self.deps, self.ctx);

        SkillTaskParams {
            call_id: call.id.clone(),
            call_name: call.name.clone(),
            call_input: call.input.clone(),
            skills: self.deps.skills.clone(),
            event_sink: self.deps.event_sink.clone(),
            secrets: self.deps.secrets.clone(),
            skill_max_output_bytes: self.agent.skill_max_output_bytes,
            skill_max_duration_ms: self.agent.skill_max_duration_ms,
            skill_timeout: self.skill_timeout,
            max_result_chars: self.max_result_chars,
            message_id,
            user_cwd,
            worktree_cwd: self.worktree_cwd.clone(),
            progress_tx,
        }
    }
}

// ---------------------------------------------------------------------------
// Skill task execution helpers
// ---------------------------------------------------------------------------

struct SkillTaskParams {
    call_id: String,
    call_name: String,
    call_input: serde_json::Value,
    skills: Arc<orka_skills::SkillRegistry>,
    event_sink: Arc<dyn EventSink>,
    secrets: Arc<dyn SecretManager>,
    skill_max_output_bytes: Option<usize>,
    skill_max_duration_ms: Option<u64>,
    skill_timeout: std::time::Duration,
    max_result_chars: usize,
    message_id: orka_core::types::MessageId,
    user_cwd: Option<String>,
    worktree_cwd: Option<String>,
    progress_tx: Option<tokio::sync::mpsc::UnboundedSender<serde_json::Value>>,
}

async fn invoke_skill_task(p: SkillTaskParams) -> (String, String, bool, Vec<MediaPayload>) {
    let args: HashMap<String, serde_json::Value> = match p.call_input {
        serde_json::Value::Object(map) => map.into_iter().collect(),
        _ => HashMap::new(),
    };
    let start = std::time::Instant::now();
    let mut skill_ctx = orka_core::SkillContext::new(p.secrets, Some(p.event_sink.clone()))
        .with_user_cwd(p.user_cwd)
        .with_worktree_cwd(p.worktree_cwd);
    if p.skill_max_output_bytes.is_some() || p.skill_max_duration_ms.is_some() {
        skill_ctx = skill_ctx.with_budget(orka_core::SkillBudget {
            max_duration_ms: p.skill_max_duration_ms,
            max_output_bytes: p.skill_max_output_bytes,
        });
    }
    if let Some(tx) = p.progress_tx {
        skill_ctx = skill_ctx.with_progress(tx);
    }
    let skill_input = SkillInput::new(args).with_context(skill_ctx);
    let result =
        match tokio::time::timeout(p.skill_timeout, p.skills.invoke(&p.call_name, skill_input))
            .await
        {
            Ok(r) => r,
            Err(_) => Err(orka_core::Error::Skill(format!(
                "skill '{}' timed out after {}s",
                p.call_name,
                p.skill_timeout.as_secs()
            ))),
        };
    let (content, is_error, task_attachments) = extract_skill_result(&result, p.max_result_chars);
    emit_skill_completed(
        &p.event_sink,
        &p.call_name,
        p.message_id,
        start.elapsed().as_millis() as u64,
        is_error,
        &result,
    )
    .await;
    (p.call_id, content, is_error, task_attachments)
}

fn extract_skill_result(
    result: &orka_core::Result<SkillOutput>,
    max_result_chars: usize,
) -> (String, bool, Vec<MediaPayload>) {
    let task_attachments: Vec<MediaPayload> = match result {
        Ok(output) => output.attachments.clone(),
        Err(_) => Vec::new(),
    };
    let (content, is_error) = match result {
        Ok(output) => {
            let raw = output.data.to_string();
            (truncate_tool_result(&raw, max_result_chars), false)
        }
        Err(e) => (format!("Error: {e}"), true),
    };
    (content, is_error, task_attachments)
}

async fn emit_skill_completed(
    event_sink: &Arc<dyn EventSink>,
    call_name: &str,
    message_id: orka_core::types::MessageId,
    duration_ms: u64,
    is_error: bool,
    result: &orka_core::Result<SkillOutput>,
) {
    let error_category = match result {
        Err(e) => Some(e.category()),
        Ok(_) => None,
    };
    let output_preview = match result {
        Ok(output) => {
            let s = output.data.to_string();
            Some(s.chars().take(1024).collect::<String>())
        }
        Err(_) => None,
    };
    let error_message = match result {
        Err(e) => Some(e.to_string()),
        Ok(_) => None,
    };
    event_sink
        .emit(DomainEvent::new(DomainEventKind::SkillCompleted {
            skill_name: call_name.to_string(),
            message_id,
            duration_ms,
            success: !is_error,
            error_category,
            output_preview,
            error_message,
        }))
        .await;
}

fn maybe_spawn_progress_bridge(
    call_name: &str,
    channel: &str,
    deps: &ExecutorDeps,
    ctx: &ExecutionContext,
) -> Option<tokio::sync::mpsc::UnboundedSender<serde_json::Value>> {
    if call_name != "coding_delegate" || channel == "custom" {
        return None;
    }
    let bus = deps.bus.as_ref()?;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let bridge_config = orka_core::progress_bridge::ProgressBridgeConfig::default();
    tokio::spawn(orka_core::progress_bridge::forward_progress_to_chat(
        rx,
        bus.clone(),
        ctx.trigger.channel.clone(),
        ctx.session_id,
        ctx.trigger.metadata.clone(),
        ctx.trigger.id,
        bridge_config,
    ));
    Some(tx)
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Execute a single agent's LLM tool loop.
///
/// This is the core of the agent system, adapted to operate on
/// `(Agent, ExecutionContext, ExecutorDeps)`.
pub(crate) async fn run_agent_node(
    agent: &Agent,
    ctx: &ExecutionContext,
    deps: &ExecutorDeps,
    graph: &AgentGraph,
) -> orka_core::Result<AgentNodeResult> {
    let Some(llm) = deps.llm.as_ref().map(Arc::clone) else {
        return Ok(AgentNodeResult {
            response: Some("No LLM provider configured.".into()),
            handoff: None,
            iterations: 0,
            interrupted: None,
            attachments: Vec::new(),
            stop_reason: orka_core::stream::AgentStopReason::Error,
        });
    };
    // Check input guardrail before initializing runner.
    let trigger_text = match &ctx.trigger.payload {
        orka_core::Payload::Text(t) => t.clone(),
        orka_core::Payload::RichInput(input) => input.text.clone().unwrap_or_default(),
        _ => String::new(),
    };
    if let Some(blocked) = check_input_guardrail(agent, ctx, deps, &trigger_text).await {
        return Ok(blocked);
    }
    AgentNodeRunner::new(agent, ctx, deps, graph, llm)
        .await?
        .run()
        .await
}

// ---------------------------------------------------------------------------
// Free helper functions
// ---------------------------------------------------------------------------

fn build_trigger_context(agent: &Agent, ctx: &ExecutionContext) -> TriggerContext {
    let _ = agent;
    let envelope = &ctx.trigger;
    let message_id = envelope.id;
    let trigger_text = match &ctx.trigger.payload {
        orka_core::Payload::Text(t) => t.clone(),
        orka_core::Payload::RichInput(input) => input.text.clone().unwrap_or_default(),
        _ => String::new(),
    };
    let workspace_name = ctx
        .trigger
        .metadata
        .get("workspace:name")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();
    let available_workspaces = ctx
        .trigger
        .metadata
        .get("workspace:available")
        .and_then(|v| v.as_array())
        .map_or_else(
            || vec![workspace_name.clone()],
            |arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            },
        );
    let cwd = ctx
        .trigger
        .metadata
        .get("workspace:cwd")
        .and_then(|v| v.as_str())
        .map(String::from);
    TriggerContext {
        trigger_text,
        workspace_name,
        available_workspaces,
        cwd,
        message_id,
    }
}

fn synthetic_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::new(
            "list_tool_categories",
            "List all available tool categories with their skills. \
             Call this first to discover what tools are available before using them.",
            serde_json::json!({"type": "object", "properties": {}}),
        ),
        ToolDefinition::new(
            "enable_tools",
            "Enable all tools from a specific category. \
             Call list_tool_categories first, then call this to activate a category.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "category": {
                        "type": "string",
                        "description": "The category name to enable (e.g. \"filesystem\", \"web\")"
                    }
                },
                "required": ["category"]
            }),
        ),
    ]
}

fn build_skill_tools(
    deps: &ExecutorDeps,
    agent: &Agent,
    progressive: bool,
    enabled: &std::collections::HashSet<String>,
) -> Vec<ToolDefinition> {
    deps.skills
        .list_available()
        .iter()
        .filter(|name| agent.tools.allows(name))
        .filter_map(|name| deps.skills.get(name))
        .filter(|skill| !progressive || enabled.contains(skill.category()))
        .map(|skill| {
            ToolDefinition::new(skill.name(), skill.description(), skill.schema().parameters)
        })
        .collect()
}

fn assemble_tools(
    progressive: bool,
    skill_tools: Vec<ToolDefinition>,
    handoff_tools: Vec<ToolDefinition>,
    plan_tools: Vec<ToolDefinition>,
) -> Vec<ToolDefinition> {
    if progressive {
        let mut t = synthetic_tools();
        t.extend(skill_tools);
        t.extend(handoff_tools);
        t.extend(plan_tools);
        t
    } else {
        let mut t = skill_tools;
        t.extend(handoff_tools);
        t.extend(plan_tools);
        t
    }
}

async fn maybe_summarize_history(ctx: &ExecutionContext, deps: &ExecutorDeps, agent: &Agent) {
    if agent.history_strategy != HistoryStrategy::Summarize {
        return;
    }
    if ctx.conversation_summary().await.is_some() {
        return;
    }
    let Some(ref llm_client) = deps.llm else {
        return;
    };
    let current_msgs = ctx.messages().await;
    if current_msgs.is_empty() {
        return;
    }
    let hint = TokenizerHint::from_model(agent.llm_config.model.as_deref());
    let context_window = agent.llm_config.context_window.unwrap_or(200_000);
    let output_budget = agent.llm_config.max_tokens.unwrap_or(4096);
    let rough_budget = context_window.saturating_sub(output_budget);
    let total: u32 = current_msgs
        .iter()
        .map(|m| orka_llm::context::estimate_message_tokens_with_hint(m, hint))
        .sum();
    if total <= rough_budget / 2 {
        return;
    }
    let turns = orka_llm::context::group_into_turns(&current_msgs);
    let cutoff = turns.len() / 2;
    if cutoff == 0 {
        return;
    }
    let end = turns[cutoff - 1].end;
    let to_summarise = &current_msgs[..end];
    match summarize_messages(llm_client.as_ref(), to_summarise).await {
        Ok(summary) => {
            debug!(agent = %agent.id, turns_summarized = cutoff, "history.summarized");
            ctx.set_conversation_summary(summary).await;
        }
        Err(e) => {
            warn!(%e, agent = %agent.id, "history summarization failed");
        }
    }
}

async fn build_agent_system_prompt(
    agent: &Agent,
    ctx: &ExecutionContext,
    deps: &ExecutorDeps,
    trigger_text: &str,
    workspace_name: &str,
    available_workspaces: &[String],
    cwd: Option<&str>,
) -> String {
    use orka_prompts::{context::SessionContext, pipeline::BuildContext};

    let soft_skill_section = build_soft_skill_section(deps, trigger_text);
    let relevant_facts_section = build_facts_section(deps, trigger_text, workspace_name, ctx).await;
    let shell_commands = build_shell_commands_section(ctx);

    let mut base_context = BuildContext::new(&agent.display_name)
        .with_persona(&agent.system_prompt.persona)
        .with_tool_instructions(&agent.system_prompt.tool_instructions)
        .with_workspace(workspace_name, available_workspaces.to_vec())
        .with_config(PipelineConfig::default());

    if let Some(cwd_val) = cwd {
        base_context = base_context.with_cwd(cwd_val.to_string());
    }
    if let Some(ref registry) = deps.templates {
        base_context = base_context.with_templates(Arc::clone(registry));
    }
    if let Some(summary) = ctx.conversation_summary().await {
        base_context = base_context.with_summary(summary);
    }

    let session_ctx = SessionContext {
        session_id: ctx.session_id.to_string(),
        workspace: workspace_name.to_string(),
        user_message: trigger_text.to_string(),
        cwd: cwd.map(String::from),
        recent_commands: vec![],
        metadata: ctx.trigger.metadata.clone(),
    };

    let mut sections = std::collections::HashMap::new();
    if !soft_skill_section.is_empty() {
        sections.insert("soft_skills".to_string(), soft_skill_section);
    }
    if !relevant_facts_section.is_empty() {
        sections.insert("relevant_facts".to_string(), relevant_facts_section);
    }
    if !shell_commands.is_empty() {
        sections.insert("shell_commands".to_string(), shell_commands);
    }
    let coordinator = build_coordinator(
        base_context,
        deps,
        workspace_name,
        available_workspaces,
        cwd,
        sections,
    );

    match coordinator.build(&session_ctx).await {
        Ok(build_ctx) => {
            let pipeline =
                orka_prompts::pipeline::SystemPromptPipeline::from_config(&build_ctx.config);
            match pipeline.build(&build_ctx).await {
                Ok(prompt) => prompt,
                Err(e) => {
                    warn!(%e, "failed to build system prompt with pipeline, using fallback");
                    format!(
                        "You are {}.\n\n{}",
                        agent.display_name, agent.system_prompt.persona
                    )
                }
            }
        }
        Err(e) => {
            warn!(%e, "failed to build context with providers, using fallback");
            format!(
                "You are {}.\n\n{}",
                agent.display_name, agent.system_prompt.persona
            )
        }
    }
}

fn build_soft_skill_section(deps: &ExecutorDeps, trigger_text: &str) -> String {
    let Some(ref soft_reg) = deps.soft_skills else {
        return String::new();
    };
    if soft_reg.is_empty() {
        return String::new();
    }
    let selected_names: Vec<&str> =
        if soft_reg.selection_mode == orka_skills::SoftSkillSelectionMode::Keyword {
            soft_reg.filter_by_message(trigger_text)
        } else {
            soft_reg.list()
        };
    soft_reg.build_prompt_section(&selected_names)
}

async fn build_facts_section(
    deps: &ExecutorDeps,
    trigger_text: &str,
    workspace_name: &str,
    ctx: &ExecutionContext,
) -> String {
    let Some(ref facts) = deps.facts else {
        return String::new();
    };
    match facts.search(trigger_text, 8, Some(0.6), None).await {
        Ok(results) => {
            let filtered: Vec<_> = results
                .into_iter()
                .filter(
                    |result| match result.metadata.get("memory_scope").map(String::as_str) {
                        Some("session") => result
                            .metadata
                            .get("session_id")
                            .is_some_and(|sid| sid == &ctx.session_id.to_string()),
                        Some("workspace") => result
                            .metadata
                            .get("workspace")
                            .is_some_and(|ws| ws == workspace_name),
                        Some("global") | None => true,
                        Some(_) => false,
                    },
                )
                .take(5)
                .collect();
            if filtered.is_empty() {
                String::new()
            } else {
                let mut lines = vec!["## Relevant Facts".to_string(), String::new()];
                for result in filtered {
                    lines.push(format!("- {}", result.content));
                }
                lines.join("\n")
            }
        }
        Err(e) => {
            warn!(%e, "failed to retrieve semantic facts");
            String::new()
        }
    }
}

fn build_shell_commands_section(ctx: &ExecutionContext) -> String {
    ctx.trigger
        .metadata
        .get("shell:recent_commands")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| format!("## Recent local shell commands\n{s}"))
        .unwrap_or_default()
}

fn build_coordinator(
    base_context: orka_prompts::context::BuildContext,
    deps: &ExecutorDeps,
    workspace_name: &str,
    available_workspaces: &[String],
    cwd: Option<&str>,
    sections: std::collections::HashMap<String, String>,
) -> orka_prompts::context::ContextCoordinator {
    use orka_prompts::context::{
        ContextCoordinator, ExperienceContextProvider, SectionsContextProvider,
        ShellContextProvider, SoftSkillsContextProvider, WorkspaceProvider,
    };

    let mut coordinator = ContextCoordinator::new(base_context);

    if let Some(ref exp) = deps.experience {
        let adapter = crate::context_adapters::ExperienceServiceAdapter::new(Arc::clone(exp));
        coordinator = coordinator.with_provider(Box::new(ExperienceContextProvider::new(
            Arc::new(adapter),
            workspace_name.to_string(),
        )));
    }

    if let Some(ref soft_reg) = deps.soft_skills {
        let adapter = crate::context_adapters::SoftSkillRegistryAdapter::new(Arc::clone(soft_reg));
        let mode = crate::context_adapters::get_soft_skill_selection_mode(soft_reg);
        coordinator = coordinator.with_provider(Box::new(SoftSkillsContextProvider::new(
            Arc::new(adapter),
            mode,
        )));
    }

    coordinator = coordinator
        .with_provider(Box::new(WorkspaceProvider::new(
            available_workspaces.to_vec(),
        )))
        .with_provider(Box::new(ShellContextProvider::new()));

    let mut all_sections = sections;
    if let Some(coding_runtime) = &deps.coding_runtime {
        all_sections.insert(
            "coding_runtime".to_string(),
            coding_runtime.render_prompt_section(cwd),
        );
    }
    if !all_sections.is_empty() {
        coordinator =
            coordinator.with_provider(Box::new(SectionsContextProvider::new(all_sections)));
    }

    coordinator
}

async fn load_history(agent: &Agent, ctx: &ExecutionContext) -> Vec<ChatMessage> {
    use orka_core::config::HistoryFilter;
    let raw = ctx.messages().await;
    let filtered = match (agent.history_filter, agent.history_filter_n) {
        (HistoryFilter::None, _) => Vec::new(),
        (HistoryFilter::LastN, Some(n)) if raw.len() > n => raw[raw.len() - n..].to_vec(),
        _ => raw,
    };
    let (sanitized, removed) = sanitize_tool_result_history(filtered);
    if removed > 0 {
        warn!(
            agent = %agent.id,
            removed_tool_results = removed,
            "dropped orphaned tool_result blocks before agent execution"
        );
    }
    sanitized
}

/// Check if input is blocked by guardrail. Returns `Some(result)` if blocked.
async fn check_input_guardrail(
    agent: &Agent,
    ctx: &ExecutionContext,
    deps: &ExecutorDeps,
    trigger_text: &str,
) -> Option<AgentNodeResult> {
    let guardrail = deps.guardrail.as_ref()?;
    let session = orka_core::Session::new(&ctx.trigger.channel, "");
    match guardrail.check_input(trigger_text, &session).await {
        Ok(GuardrailDecision::Block(reason)) => {
            warn!(agent = %agent.id, %reason, "input blocked by guardrail");
            Some(AgentNodeResult {
                response: Some(format!("Input blocked: {reason}")),
                handoff: None,
                iterations: 0,
                interrupted: None,
                attachments: Vec::new(),
                stop_reason: orka_core::stream::AgentStopReason::Error,
            })
        }
        _ => None,
    }
}

/// Apply guardrail modification to the last user message (Modify decision
/// only).
async fn apply_input_guardrail_modify(
    deps: &ExecutorDeps,
    ctx: &ExecutionContext,
    trigger_text: &str,
    mut messages: Vec<ChatMessage>,
) -> Vec<ChatMessage> {
    let Some(ref guardrail) = deps.guardrail else {
        return messages;
    };
    let session = orka_core::Session::new(&ctx.trigger.channel, "");
    if let Ok(GuardrailDecision::Modify(filtered)) =
        guardrail.check_input(trigger_text, &session).await
        && let Some(last) = messages.last_mut()
        && matches!(last.role, orka_llm::client::Role::User)
    {
        *last = orka_llm::client::ChatMessage::user(filtered);
    }
    messages
}

async fn maybe_generate_plan(
    agent: &Agent,
    ctx: &ExecutionContext,
    deps: &ExecutorDeps,
    trigger_text: &str,
    mut system_prompt: String,
) -> String {
    if agent.planning_mode != PlanningMode::Always {
        return system_prompt;
    }
    if ctx.get(&SlotKey::shared(PLAN_SLOT)).await.is_some() {
        return system_prompt;
    }
    let Some(ref llm_client) = deps.llm else {
        return system_prompt;
    };
    // Load messages for plan context
    let messages = ctx.messages().await;
    match generate_plan(llm_client.as_ref(), trigger_text, &messages).await {
        Ok(plan) => {
            let plan_section = format!("\n\n## Task Plan\n{}", plan.display_summary());
            system_prompt.push_str(&plan_section);
            if let Ok(plan_json) = serde_json::to_value(&plan) {
                ctx.set(&agent.id, SlotKey::shared(PLAN_SLOT), plan_json)
                    .await;
            }
            debug!(agent = %agent.id, steps = plan.steps.len(), "always plan generated");
        }
        Err(e) => {
            warn!(%e, agent = %agent.id, "PlanningMode::Always plan generation failed, continuing without plan");
        }
    }
    system_prompt
}

async fn parse_completion_blocks(
    agent: &Agent,
    deps: &ExecutorDeps,
    completion: &CompletionResponse,
    message_id: orka_core::types::MessageId,
) -> (String, Vec<ToolCall>) {
    let mut thinking_text = String::new();
    let mut response_text = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    for block in &completion.blocks {
        match block {
            ContentBlock::Thinking(t) => thinking_text.push_str(t),
            ContentBlock::Text(t) => response_text.push_str(t),
            ContentBlock::ToolUse(call) => tool_calls.push(call.clone()),
            _ => debug!("unhandled content block"),
        }
    }

    if !thinking_text.is_empty() {
        deps.event_sink
            .emit(DomainEvent::new(DomainEventKind::AgentReasoning {
                message_id,
                iteration: 0,
                reasoning_text: thinking_text,
            }))
            .await;
    }
    let _ = agent;
    (response_text, tool_calls)
}

async fn emit_llm_completed_event(
    deps: &ExecutorDeps,
    agent: &Agent,
    message_id: orka_core::types::MessageId,
    completion: &CompletionResponse,
    llm_duration_ms: u64,
) -> String {
    let llm_model = agent
        .llm_config
        .model
        .clone()
        .unwrap_or_else(|| "default".into());
    deps.event_sink
        .emit(DomainEvent::new(DomainEventKind::LlmCompleted {
            message_id,
            model: llm_model.clone(),
            provider: infer_provider(&llm_model),
            input_tokens: completion.usage.input_tokens,
            output_tokens: completion.usage.output_tokens,
            reasoning_tokens: completion.usage.reasoning_tokens,
            duration_ms: llm_duration_ms,
            estimated_cost_usd: None,
        }))
        .await;
    llm_model
}

async fn emit_iteration_event(
    deps: &ExecutorDeps,
    message_id: orka_core::types::MessageId,
    iteration: usize,
    tool_count: usize,
    tokens_used: u64,
    start: &std::time::Instant,
) {
    deps.event_sink
        .emit(DomainEvent::new(DomainEventKind::AgentIteration {
            message_id,
            iteration,
            tool_count,
            tokens_used,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }))
        .await;
}

fn push_handoff_assistant_message(
    messages: &mut Vec<ChatMessage>,
    response_text: &str,
    hc: &ToolCall,
) {
    let mut blocks = Vec::new();
    if !response_text.is_empty() {
        blocks.push(ContentBlockInput::Text {
            text: response_text.to_string(),
        });
    }
    blocks.push(ContentBlockInput::ToolUse {
        id: hc.id.clone(),
        name: hc.name.clone(),
        input: hc.input.clone(),
    });
    if !blocks.is_empty() {
        messages.push(ChatMessage::new(
            orka_llm::client::Role::Assistant,
            ChatContent::Blocks(blocks),
        ));
    }
}

fn push_tool_call_assistant_message(
    messages: &mut Vec<ChatMessage>,
    response_text: &str,
    regular_calls: &[ToolCall],
) {
    let mut blocks = Vec::new();
    if !response_text.is_empty() {
        blocks.push(ContentBlockInput::Text {
            text: response_text.to_string(),
        });
    }
    for call in regular_calls {
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

async fn handle_plan_tool(
    agent: &Agent,
    ctx: &ExecutionContext,
    call: &ToolCall,
) -> Option<(String, bool)> {
    if call.name != "create_plan" && call.name != "update_plan_step" {
        return None;
    }
    let result = match call.name.as_str() {
        "create_plan" => {
            let goal = call
                .input
                .get("goal")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let steps: Vec<PlanStep> = call
                .input
                .get("steps")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| {
                            let id = s.get("id")?.as_str()?.to_string();
                            let description = s.get("description")?.as_str()?.to_string();
                            Some(PlanStep {
                                id,
                                description,
                                status: StepStatus::Pending,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let plan = Plan { goal, steps };
            let summary = plan.display_summary();
            if let Ok(json) = serde_json::to_value(&plan) {
                ctx.set(&agent.id, SlotKey::shared(PLAN_SLOT), json).await;
            }
            (format!("Plan created.\n{summary}"), false)
        }
        "update_plan_step" => execute_update_plan_step(agent, ctx, call).await,
        other => {
            tracing::warn!(call_name = %other, "unexpected plan-related tool call — skipped");
            (format!("Error: unrecognized plan call '{other}'"), true)
        }
    };
    Some(result)
}

async fn execute_update_plan_step(
    agent: &Agent,
    ctx: &ExecutionContext,
    call: &ToolCall,
) -> (String, bool) {
    let step_id = call
        .input
        .get("step_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let status_str = call
        .input
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("completed");
    let summary = call
        .input
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let plan_key = SlotKey::shared(PLAN_SLOT);
    let updated = if let Some(json) = ctx.get(&plan_key).await {
        match serde_json::from_value::<Plan>(json) {
            Ok(mut plan) => {
                if let Some(step) = plan.steps.iter_mut().find(|s| s.id == step_id) {
                    step.status = match status_str {
                        "in_progress" => StepStatus::InProgress,
                        "failed" => StepStatus::Failed { summary },
                        "skipped" => StepStatus::Skipped { summary },
                        _ => StepStatus::Completed { summary },
                    };
                }
                let s = plan.display_summary();
                if let Ok(json) = serde_json::to_value(&plan) {
                    ctx.set(&agent.id, plan_key, json).await;
                }
                s
            }
            Err(_) => "Error: plan data is corrupt.".to_string(),
        }
    } else {
        "Error: no active plan. Call create_plan first.".to_string()
    };
    (updated, false)
}

fn handle_progressive_tool(
    call: &ToolCall,
    progressive: bool,
    enabled_categories: &mut std::collections::HashSet<String>,
) -> Option<(String, bool)> {
    if !progressive {
        return None;
    }
    if call.name != "list_tool_categories" && call.name != "enable_tools" {
        return None;
    }
    // These are handled inline — no deps needed
    // Note: list_tool_categories needs deps.skills but we handle it via a
    // placeholder and let dispatch_tool_calls call it with deps available.
    // Actually we return None for list_tool_categories to let the caller handle it.
    // We only handle enable_tools here since it only mutates enabled_categories.
    if call.name == "enable_tools" {
        let cat = call
            .input
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if cat.is_empty() {
            return Some(("Error: 'category' parameter is required".to_string(), true));
        }
        enabled_categories.insert(cat.clone());
        return Some((format!("Tools in category '{cat}' are now enabled."), false));
    }
    None
}

async fn apply_tool_guardrails(
    agent: &Agent,
    ctx: &ExecutionContext,
    deps: &ExecutorDeps,
    skill_calls: Vec<ToolCall>,
) -> (Vec<ToolCall>, HashMap<String, (String, bool)>) {
    let mut checked = Vec::new();
    let mut blocked = HashMap::new();
    let Some(ref guardrail) = deps.guardrail else {
        return (skill_calls, blocked);
    };
    for mut call in skill_calls {
        let session = orka_core::Session::new(&ctx.trigger.channel, "");
        let input_json = call.input.to_string();
        match guardrail.check_input(&input_json, &session).await {
            Ok(GuardrailDecision::Block(reason)) => {
                warn!(skill = %call.name, %reason, "tool input blocked by guardrail");
                blocked.insert(
                    call.id.clone(),
                    (format!("Tool input blocked by guardrail: {reason}"), true),
                );
            }
            Ok(GuardrailDecision::Modify(modified)) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&modified) {
                    call.input = v;
                }
                checked.push(call);
            }
            _ => {
                checked.push(call);
            }
        }
        let _ = agent;
    }
    (checked, blocked)
}

fn update_worktree_cwd(
    regular_calls: &[ToolCall],
    results_map: &HashMap<String, (String, bool)>,
    worktree_cwd: &mut Option<String>,
) {
    for call in regular_calls {
        if let Some((content, false)) = results_map.get(&call.id) {
            match call.name.as_str() {
                "git_worktree_create" => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(content)
                        && let Some(path) = v.get("path").and_then(|p| p.as_str())
                    {
                        *worktree_cwd = Some(path.to_string());
                        debug!(worktree_path = path, "worktree context activated");
                    }
                }
                "git_worktree_remove" => {
                    *worktree_cwd = None;
                    debug!("worktree context cleared");
                }
                _ => {}
            }
        }
    }
}

fn build_result_blocks(
    regular_calls: &[ToolCall],
    results_map: &mut HashMap<String, (String, bool)>,
) -> Vec<ContentBlockInput> {
    regular_calls
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
        .collect()
}

async fn apply_output_guardrail(
    agent: &Agent,
    ctx: &ExecutionContext,
    deps: &ExecutorDeps,
    final_response: Option<String>,
) -> Option<String> {
    let text = final_response?;
    let Some(guardrail) = &deps.guardrail else {
        return Some(text);
    };
    let session = orka_core::Session::new(&ctx.trigger.channel, "");
    match guardrail.check_output(&text, &session).await {
        Ok(GuardrailDecision::Allow) | Err(_) => Some(text),
        Ok(GuardrailDecision::Block(reason)) => {
            warn!(agent = %agent.id, %reason, "output blocked by guardrail");
            Some(format!("Response blocked by content policy: {reason}"))
        }
        Ok(GuardrailDecision::Modify(filtered)) => Some(filtered),
        Ok(other) => {
            warn!(?other, "unhandled guardrail decision, passing through");
            Some(text)
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Call the LLM to produce a brief summary of `messages`.
///
/// Used by `HistoryStrategy::Summarize` to compress old turns before they are
/// dropped from the context window.  Returns an error on any LLM failure; the
/// caller logs and falls back to plain truncation.
async fn summarize_messages(
    llm: &dyn orka_llm::client::LlmClient,
    messages: &[orka_llm::client::ChatMessage],
) -> orka_core::Result<String> {
    use orka_llm::{
        client::{ChatMessage, CompletionOptions},
        consume_stream,
    };

    let system = "You are a concise conversation summarizer. \
                  Summarize the key information, decisions, tool results, and context \
                  from the provided conversation in 3-7 sentences. \
                  Focus on facts the assistant will need later; omit pleasantries.";

    // Format messages as plain text for summarization.
    let transcript: String = messages
        .iter()
        .map(|m| {
            let role = match m.role {
                orka_llm::client::Role::User => "User",
                orka_llm::client::Role::Assistant => "Assistant",
                _ => "Unknown",
            };
            let text = match &m.content {
                orka_llm::client::ChatContent::Text(t) => t.clone(),
                orka_llm::client::ChatContent::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|b| match b {
                        orka_llm::client::ContentBlockInput::Text { text } => Some(text.as_str()),
                        orka_llm::client::ContentBlockInput::ToolResult { content, .. } => {
                            Some(content.as_str())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => String::new(),
            };
            format!("{role}: {text}")
        })
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!("Conversation to summarize:\n\n{transcript}");
    let msgs = vec![ChatMessage::user(prompt)];

    let stream = llm
        .complete_stream_with_tools(&msgs, system, &[], &CompletionOptions::default())
        .await
        .map_err(|e| orka_core::Error::llm(e, "llm summarize stream"))?;

    // Use a no-op stream registry for the summarization call.
    let registry = orka_core::StreamRegistry::new();
    let session_id = orka_core::SessionId::new();
    let completion = consume_stream(stream, &session_id, &registry, "summarize", None)
        .await
        .map_err(|e| orka_core::Error::llm(e, "llm summarize consume"))?;

    let text = completion
        .blocks
        .iter()
        .filter_map(|b| match b {
            orka_llm::client::ContentBlock::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");

    Ok(text)
}

/// Call the LLM to generate a structured [`Plan`] for a task.
///
/// Used by [`PlanningMode::Always`] to proactively plan before the first
/// iteration.  Returns an error on any LLM failure; the caller logs and
/// continues without a plan.
async fn generate_plan(
    llm: &dyn orka_llm::client::LlmClient,
    trigger_text: &str,
    messages: &[orka_llm::client::ChatMessage],
) -> orka_core::Result<Plan> {
    use orka_llm::client::{ChatMessage, CompletionOptions, ResponseFormat};

    #[derive(serde::Deserialize)]
    struct RawPlan {
        goal: String,
        steps: Vec<RawStep>,
    }
    #[derive(serde::Deserialize)]
    struct RawStep {
        id: String,
        description: String,
    }

    let system = "You are a task planning assistant. Given a user request and conversation \
                  context, produce a concise step-by-step plan in JSON. \
                  Respond ONLY with valid JSON matching: \
                  {\"goal\": \"...\", \"steps\": [{\"id\": \"s1\", \"description\": \"...\"}]}";

    let context_snippet: String = messages
        .iter()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .filter_map(|m| match &m.content {
            orka_llm::client::ChatContent::Text(t) if !t.is_empty() => Some(t.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = if context_snippet.is_empty() {
        format!("Task: {trigger_text}")
    } else {
        format!("Recent context:\n{context_snippet}\n\nTask: {trigger_text}")
    };

    let msgs = vec![ChatMessage::user(prompt)];
    let mut opts = CompletionOptions::default();
    opts.max_tokens = Some(512);
    opts.response_format = Some(ResponseFormat::Json);

    let raw = llm
        .complete_with_options(msgs, system, &opts)
        .await
        .map_err(|e| orka_core::Error::llm(e, "llm plan completion"))?;

    // Parse response — allow partial structures gracefully
    let raw_plan: RawPlan = serde_json::from_str(&raw)
        .map_err(|e| orka_core::Error::llm(e, format!("plan parse error — raw: {raw}")))?;

    Ok(Plan {
        goal: raw_plan.goal,
        steps: raw_plan
            .steps
            .into_iter()
            .map(|s| PlanStep {
                id: s.id,
                description: s.description,
                status: StepStatus::Pending,
            })
            .collect(),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use orka_core::{
        Envelope, SessionId, StreamRegistry,
        testing::{InMemoryEventSink, InMemoryMemoryStore, InMemorySecretManager},
    };
    use orka_llm::{
        client::StopReason,
        testing::{CompletionResponseBuilder, MockLlmClient},
    };
    use orka_skills::SkillRegistry;

    use super::*;
    use crate::{
        agent::{Agent, AgentId},
        context::ExecutionContext,
        executor::ExecutorDeps,
        graph::AgentGraph,
        handoff::HandoffMode,
    };

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn minimal_deps(mock: MockLlmClient) -> ExecutorDeps {
        ExecutorDeps {
            skills: Arc::new(SkillRegistry::new()),
            memory: Arc::new(InMemoryMemoryStore::new()),
            secrets: Arc::new(InMemorySecretManager::new()),
            llm: Some(Arc::new(mock)),
            event_sink: Arc::new(InMemoryEventSink::new()),
            stream_registry: StreamRegistry::new(),
            experience: None,
            facts: None,
            soft_skills: None,
            templates: None,
            coding_runtime: None,
            guardrail: None,
            checkpoint_store: None,
            bus: None,
        }
    }

    fn minimal_ctx() -> ExecutionContext {
        ExecutionContext::new(Envelope::text("ch", SessionId::new(), "hello"))
    }

    fn minimal_graph(agent_id: &AgentId) -> AgentGraph {
        AgentGraph::new("test-graph", agent_id.clone())
    }

    // ── Unit tests: pure helpers ──────────────────────────────────────────────

    #[test]
    fn parse_handoff_sets_transfer_mode() {
        let from = AgentId::new("a");
        let call = orka_llm::client::ToolCall::new(
            "id1",
            "transfer_to_agent",
            serde_json::json!({"agent_id": "b", "reason": "needs it"}),
        );
        let h = parse_handoff(&from, &call);
        assert_eq!(h.from, from);
        assert_eq!(h.to, AgentId::from("b"));
        assert_eq!(h.reason, "needs it");
        assert_eq!(h.mode, HandoffMode::Transfer);
    }

    #[test]
    fn parse_handoff_sets_delegate_mode() {
        let from = AgentId::new("a");
        let call = orka_llm::client::ToolCall::new(
            "id2",
            "delegate_to_agent",
            serde_json::json!({"agent_id": "c", "reason": "sub-task"}),
        );
        let h = parse_handoff(&from, &call);
        assert_eq!(h.mode, HandoffMode::Delegate);
        assert_eq!(h.to, AgentId::from("c"));
    }

    #[test]
    fn parse_handoff_transfers_context_map() {
        let from = AgentId::new("a");
        let call = orka_llm::client::ToolCall::new(
            "id3",
            "transfer_to_agent",
            serde_json::json!({
                "agent_id": "b",
                "reason": "r",
                "context": {"key": "value"}
            }),
        );
        let h = parse_handoff(&from, &call);
        assert_eq!(
            h.context_transfer.get("key"),
            Some(&serde_json::json!("value"))
        );
    }

    #[test]
    fn build_tool_error_hint_returns_none_below_threshold() {
        let mut counts = HashMap::new();
        counts.insert("my_tool".to_string(), 1u32);
        assert!(build_tool_error_hint(&counts, 2).is_none());
    }

    #[test]
    fn build_tool_error_hint_returns_some_at_threshold() {
        let mut counts = HashMap::new();
        counts.insert("my_tool".to_string(), 2u32);
        let hint = build_tool_error_hint(&counts, 2).expect("expected hint");
        assert!(hint.contains("my_tool"));
        assert!(hint.contains('2'));
    }

    #[test]
    fn build_tool_error_hint_returns_none_for_empty_counts() {
        assert!(build_tool_error_hint(&HashMap::new(), 2).is_none());
    }

    // ── Integration tests: run_agent_node ────────────────────────────────────

    #[tokio::test]
    async fn simple_text_response() {
        let mock = MockLlmClient::new().with_tool_response(
            CompletionResponseBuilder::new()
                .text("Hello from mock!")
                .stop_reason(StopReason::EndTurn)
                .build(),
        );
        let agent = Agent::new(AgentId::new("test"), "Test");
        let ctx = minimal_ctx();
        let deps = minimal_deps(mock);
        let graph = minimal_graph(&agent.id);

        let result = run_agent_node(&agent, &ctx, &deps, &graph).await.unwrap();

        assert_eq!(result.response, Some("Hello from mock!".to_string()));
        assert!(result.handoff.is_none());
        assert_eq!(result.iterations, 1);
    }

    #[tokio::test]
    async fn no_llm_returns_fallback_message() {
        let deps = ExecutorDeps {
            skills: Arc::new(SkillRegistry::new()),
            memory: Arc::new(InMemoryMemoryStore::new()),
            secrets: Arc::new(InMemorySecretManager::new()),
            llm: None,
            event_sink: Arc::new(InMemoryEventSink::new()),
            stream_registry: StreamRegistry::new(),
            experience: None,
            facts: None,
            soft_skills: None,
            templates: None,
            coding_runtime: None,
            guardrail: None,
            checkpoint_store: None,
            bus: None,
        };
        let agent = Agent::new(AgentId::new("test"), "Test");
        let ctx = minimal_ctx();
        let graph = minimal_graph(&agent.id);

        let result = run_agent_node(&agent, &ctx, &deps, &graph).await.unwrap();

        assert!(result.response.is_some());
        assert_eq!(result.iterations, 0);
    }

    #[tokio::test]
    async fn handoff_detection_transfer() {
        let mock = MockLlmClient::new().with_tool_response(
            CompletionResponseBuilder::new()
                .tool_use(
                    "hid1",
                    "transfer_to_agent",
                    serde_json::json!({"agent_id": "specialist", "reason": "complex query"}),
                )
                .stop_reason(StopReason::ToolUse)
                .build(),
        );
        let agent = Agent::new(AgentId::new("main"), "Main");
        let ctx = minimal_ctx();
        let deps = minimal_deps(mock);
        let graph = minimal_graph(&agent.id);

        let result = run_agent_node(&agent, &ctx, &deps, &graph).await.unwrap();

        let handoff = result.handoff.expect("expected a handoff");
        assert_eq!(handoff.to, AgentId::from("specialist"));
        assert_eq!(handoff.reason, "complex query");
        assert_eq!(handoff.mode, HandoffMode::Transfer);
    }

    #[tokio::test]
    async fn max_turns_cap() {
        // Queue enough tool-call responses to saturate max_turns.
        // The skill registry is empty so every tool call returns an error,
        // keeping the loop running until it hits the cap.
        let tool_resp = CompletionResponseBuilder::new()
            .tool_use("id1", "nonexistent", serde_json::json!({}))
            .stop_reason(StopReason::ToolUse)
            .build();
        let mock = (0..5).fold(MockLlmClient::new(), |m, _| {
            m.with_tool_response(tool_resp.clone())
        });

        let mut agent = Agent::new(AgentId::new("test"), "Test");
        agent.max_turns = 3;

        let ctx = minimal_ctx();
        let deps = minimal_deps(mock);
        let graph = minimal_graph(&agent.id);

        let result = run_agent_node(&agent, &ctx, &deps, &graph).await.unwrap();

        assert_eq!(result.iterations, 3);
        assert_eq!(
            result.stop_reason,
            orka_core::stream::AgentStopReason::MaxTurns
        );
    }

    /// `PlanningMode::Always` triggers a pre-execution plan generation call and
    /// injects the plan into the context before the first LLM iteration.
    #[tokio::test]
    async fn planning_mode_always_generates_plan_before_execution() {
        use orka_llm::{client::StopReason, testing::CompletionResponseBuilder};

        let plan_json =
            r#"{"goal": "do the thing", "steps": [{"id": "s1", "description": "step one"}]}"#;

        // First call (complete_with_options → complete): plan generation.
        // Second call (complete_with_tools): main agent response.
        let mock = MockLlmClient::new()
            .with_text_response(plan_json)
            .with_tool_response(
                CompletionResponseBuilder::new()
                    .text("done")
                    .stop_reason(StopReason::EndTurn)
                    .build(),
            );

        let mut agent = Agent::new(AgentId::new("planner"), "Planner");
        agent.planning_mode = PlanningMode::Always;

        let ctx = minimal_ctx();
        let deps = minimal_deps(mock);
        let graph = minimal_graph(&agent.id);

        let result = run_agent_node(&agent, &ctx, &deps, &graph).await.unwrap();

        // Agent should complete with the final text response
        assert_eq!(result.response.unwrap_or_default(), "done");

        // The plan must have been stored in the shared context slot
        let plan_value = ctx.get(&SlotKey::shared(PLAN_SLOT)).await;
        assert!(
            plan_value.is_some(),
            "plan should be stored in context under PLAN_SLOT"
        );
        let plan_obj = plan_value.unwrap();
        assert_eq!(
            plan_obj.get("goal").and_then(|v| v.as_str()),
            Some("do the thing")
        );
    }

    /// `HistoryStrategy::RollingWindow` drops old turns and stores a summary
    /// in the context when the message history exceeds `recent_turns`.
    #[tokio::test]
    async fn rolling_window_trims_history_and_stores_summary() {
        use orka_llm::{
            client::{ChatMessage, Role, StopReason},
            testing::CompletionResponseBuilder,
        };

        // Two historical turns (user+assistant pairs) already in context.
        // With recent_turns = 1 the first turn will be dropped and summarized.
        let ctx = minimal_ctx();
        ctx.push_message(ChatMessage::new(
            Role::User,
            orka_llm::client::ChatContent::Text("hi".into()),
        ))
        .await;
        ctx.push_message(ChatMessage::new(
            Role::Assistant,
            orka_llm::client::ChatContent::Text("hello".into()),
        ))
        .await;
        ctx.push_message(ChatMessage::new(
            Role::User,
            orka_llm::client::ChatContent::Text("follow up".into()),
        ))
        .await;
        ctx.push_message(ChatMessage::new(
            Role::Assistant,
            orka_llm::client::ChatContent::Text("answer".into()),
        ))
        .await;

        // First tool response: summary of dropped turn.
        // Second tool response: final agent answer.
        let mock = MockLlmClient::new()
            .with_tool_response(
                CompletionResponseBuilder::new()
                    .text("prior context summary")
                    .stop_reason(StopReason::EndTurn)
                    .build(),
            )
            .with_tool_response(
                CompletionResponseBuilder::new()
                    .text("final answer")
                    .stop_reason(StopReason::EndTurn)
                    .build(),
            );

        let mut agent = Agent::new(AgentId::new("rw"), "RW");
        agent.history_strategy = HistoryStrategy::RollingWindow { recent_turns: 1 };

        let deps = minimal_deps(mock);
        let graph = minimal_graph(&agent.id);

        let result = run_agent_node(&agent, &ctx, &deps, &graph).await.unwrap();

        assert_eq!(result.response.unwrap_or_default(), "final answer");

        // The summary of the dropped turn must be stored in context.
        let summary = ctx.conversation_summary().await;
        assert!(
            summary.is_some(),
            "rolling window should store a conversation summary"
        );
        assert_eq!(summary.as_deref(), Some("prior context summary"));
    }
}
