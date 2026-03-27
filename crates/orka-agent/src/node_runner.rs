//! Single-agent LLM tool loop, extracted from `workspace_handler.rs`.

use std::{collections::HashMap, sync::Arc};

use orka_core::{
    DomainEvent, DomainEventKind, SkillInput, truncate_tool_result, types::MediaPayload,
};
use orka_llm::{
    client::{
        ChatContent, ChatMessage, CompletionOptions, ContentBlock, ContentBlockInput, ToolCall,
        ToolDefinition,
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

/// Execute a single agent's LLM tool loop.
///
/// This is the core of the agent system, adapted to operate on
/// `(Agent, ExecutionContext, ExecutorDeps)`.
#[allow(clippy::too_many_lines)]
pub(crate) async fn run_agent_node(
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
                interrupted: None,
                attachments: Vec::new(),
                stop_reason: orka_core::stream::AgentStopReason::Error,
            });
        }
    };

    // B1: Progressive disclosure state
    let progressive = agent.progressive_disclosure;
    let mut enabled_categories: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    // Synthetic tool definitions for progressive disclosure
    let synthetic_tools = || -> Vec<ToolDefinition> {
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
    };

    // Build the initial tool list
    let build_skill_tools = |enabled: &std::collections::HashSet<String>| -> Vec<ToolDefinition> {
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
    };

    let handoff_tools = build_handoff_tools(agent, graph);
    let plan_tools = if agent.planning_mode == PlanningMode::Adaptive {
        planning_tools()
    } else {
        vec![]
    };

    let initial_skill_tools = build_skill_tools(&enabled_categories);
    let mut tools: Vec<ToolDefinition> = if progressive {
        let mut t = synthetic_tools();
        t.extend(initial_skill_tools);
        t.extend(handoff_tools.clone());
        t.extend(plan_tools.clone());
        t
    } else {
        let mut t = initial_skill_tools;
        t.extend(handoff_tools.clone());
        t.extend(plan_tools.clone());
        t
    };

    let envelope = &ctx.trigger;
    let message_id = envelope.id;

    // Extract trigger text once — used for principle retrieval and soft skill
    // selection.
    let trigger_text = match &ctx.trigger.payload {
        orka_core::Payload::Text(t) => t.clone(),
        _ => String::new(),
    };

    // Get workspace info
    let workspace_name = ctx
        .trigger
        .metadata
        .get("workspace:name")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let available_workspaces = ctx
        .trigger
        .metadata
        .get("workspace:available")
        .and_then(|v| v.as_array())
        .map_or_else(
            || vec![workspace_name.to_string()],
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

    // Retrieve principles if experience is available
    let principles = if let Some(ref exp) = deps.experience {
        match exp
            .retrieve_principles(&trigger_text, agent.id.as_str())
            .await
        {
            Ok(principles) if !principles.is_empty() => {
                // Emit events for principles injection
                deps.event_sink
                    .emit(DomainEvent::new(DomainEventKind::PrinciplesInjected {
                        session_id: ctx.session_id,
                        count: principles.len(),
                    }))
                    .await;
                deps.stream_registry
                    .send(orka_core::stream::StreamChunk::new(
                        ctx.session_id,
                        envelope.channel.clone(),
                        Some(envelope.id),
                        orka_core::stream::StreamChunkKind::PrinciplesUsed {
                            count: principles.len() as u32,
                        },
                    ));
                // Convert principles to JSON for the pipeline
                principles
                    .into_iter()
                    .map(|p| {
                        serde_json::json!({
                            "text": p.text,
                            "kind": match p.kind {
                                orka_experience::types::PrincipleKind::Do => "do",
                                orka_experience::types::PrincipleKind::Avoid => "avoid",
                            },
                        })
                    })
                    .collect()
            }
            Err(e) => {
                warn!(%e, "failed to retrieve principles");
                vec![]
            }
            _ => vec![],
        }
    } else {
        vec![]
    };

    let relevant_facts_section = if let Some(ref facts) = deps.facts {
        match facts.search(&trigger_text, 8, Some(0.6), None).await {
            Ok(results) => {
                let filtered: Vec<_> = results
                    .into_iter()
                    .filter(|result| {
                        match result.metadata.get("memory_scope").map(String::as_str) {
                            Some("session") => result
                                .metadata
                                .get("session_id")
                                .is_some_and(|sid| sid == &ctx.session_id.to_string()),
                            Some("workspace") => result
                                .metadata
                                .get("workspace")
                                .is_some_and(|ws| ws == workspace_name),
                            Some("global") | None => true,
                            Some("user") => false,
                            Some(_) => false,
                        }
                    })
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
    } else {
        String::new()
    };

    // Get soft skill section
    let soft_skill_section = if let Some(ref soft_reg) = deps.soft_skills {
        if soft_reg.is_empty() {
            String::new()
        } else {
            let selected_names: Vec<&str> =
                if soft_reg.selection_mode == orka_skills::SoftSkillSelectionMode::Keyword {
                    soft_reg.filter_by_message(&trigger_text)
                } else {
                    soft_reg.list()
                };
            soft_reg.build_prompt_section(&selected_names)
        }
    } else {
        String::new()
    };

    // Get shell commands context
    let shell_commands = ctx
        .trigger
        .metadata
        .get("shell:recent_commands")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| format!("## Recent local shell commands\n{s}"))
        .unwrap_or_default();

    // ── History summarization (HistoryStrategy::Summarize) ───────────────────
    // Before building the system prompt we check if history truncation would
    // occur.  If so, and if the agent uses the Summarize strategy, we call the
    // LLM once to summarise the would-be-dropped turns and store the result in
    // `ctx` so it can be injected into the system prompt and persisted across
    // checkpoints.
    if agent.history_strategy == HistoryStrategy::Summarize
        && ctx.conversation_summary().await.is_none()
        && let Some(ref llm_client) = deps.llm
    {
        {
            let current_msgs = ctx.messages().await;
            if !current_msgs.is_empty() {
                let hint = TokenizerHint::from_model(agent.llm_config.model.as_deref());
                // Use a conservative budget estimate (no tools/system yet).
                let context_window = agent.llm_config.context_window.unwrap_or(200_000);
                let output_budget = agent.llm_config.max_tokens.unwrap_or(4096);
                let rough_budget = context_window.saturating_sub(output_budget);
                let total: u32 = current_msgs
                    .iter()
                    .map(|m| orka_llm::context::estimate_message_tokens_with_hint(m, hint))
                    .sum();
                if total > rough_budget / 2 {
                    // History is large enough that truncation is likely.
                    // Summarise oldest half of turns.
                    let turns = orka_llm::context::group_into_turns(&current_msgs);
                    let cutoff = turns.len() / 2;
                    if cutoff > 0 {
                        let end = turns[cutoff - 1].end;
                        let to_summarise = &current_msgs[..end];
                        let summary_result =
                            summarize_messages(llm_client.as_ref(), to_summarise).await;
                        match summary_result {
                            Ok(summary) => {
                                debug!(
                                    agent = %agent.id,
                                    turns_summarized = cutoff,
                                    "history.summarized"
                                );
                                ctx.set_conversation_summary(summary).await;
                            }
                            Err(e) => {
                                warn!(%e, agent = %agent.id, "history summarization failed");
                            }
                        }
                    }
                }
            }
        }
    }

    // Build system prompt using context providers and pipeline
    let mut system_prompt = {
        use orka_prompts::{
            context::{
                ContextCoordinator, ExperienceContextProvider, SectionsContextProvider,
                SessionContext, ShellContextProvider, SoftSkillsContextProvider, WorkspaceProvider,
            },
            pipeline::BuildContext,
        };

        // 1. Create base context using the unified BuildContext
        let mut base_context = BuildContext::new(&agent.display_name)
            .with_persona(&agent.system_prompt.persona)
            .with_tool_instructions(&agent.system_prompt.tool_instructions)
            .with_workspace(workspace_name, available_workspaces.clone())
            .with_config(PipelineConfig::default());

        if let Some(cwd_val) = cwd.clone() {
            base_context = base_context.with_cwd(cwd_val);
        }
        if !principles.is_empty() {
            base_context = base_context.with_principles(principles);
        }
        if let Some(ref registry) = deps.templates {
            base_context = base_context.with_templates(Arc::clone(registry));
        }
        if let Some(summary) = ctx.conversation_summary().await {
            base_context = base_context.with_summary(summary);
        }

        // 2. Create session context
        let session_ctx = SessionContext {
            session_id: ctx.session_id.to_string(),
            workspace: workspace_name.to_string(),
            user_message: trigger_text.clone(),
            cwd: cwd.clone(),
            recent_commands: vec![],
            metadata: ctx.trigger.metadata.clone(),
        };

        // 3. Configure coordinator with providers
        let mut coordinator = ContextCoordinator::new(base_context);

        // Add experience provider if available
        if let Some(ref exp) = deps.experience {
            let adapter = crate::context_adapters::ExperienceServiceAdapter::new(Arc::clone(exp));
            coordinator = coordinator.with_provider(Box::new(ExperienceContextProvider::new(
                Arc::new(adapter),
                workspace_name.to_string(),
            )));
        }

        // Add soft skills provider if available
        if let Some(ref soft_reg) = deps.soft_skills {
            let adapter =
                crate::context_adapters::SoftSkillRegistryAdapter::new(Arc::clone(soft_reg));
            let mode = crate::context_adapters::get_soft_skill_selection_mode(soft_reg);
            coordinator = coordinator.with_provider(Box::new(SoftSkillsContextProvider::new(
                Arc::new(adapter),
                mode,
            )));
        }

        // Add workspace and shell providers
        coordinator = coordinator
            .with_provider(Box::new(WorkspaceProvider::new(
                available_workspaces.clone(),
            )))
            .with_provider(Box::new(ShellContextProvider::new()));

        // Add dynamic sections provider for soft skills and shell commands
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
        if let Some(coding_runtime) = &deps.coding_runtime {
            let user_cwd = ctx
                .trigger
                .metadata
                .get("workspace:cwd")
                .and_then(|value| value.as_str());
            sections.insert(
                "coding_runtime".to_string(),
                coding_runtime.render_prompt_section(user_cwd),
            );
        }
        if !sections.is_empty() {
            coordinator =
                coordinator.with_provider(Box::new(SectionsContextProvider::new(sections)));
        }

        // 4. Build context and render prompt
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
    };

    let mut options = CompletionOptions::default();
    options.model = agent.llm_config.model.clone();
    options.max_tokens = agent.llm_config.max_tokens;
    options.temperature = agent.llm_config.temperature;
    options.thinking = agent.llm_config.thinking.clone();

    let context_window = agent.llm_config.context_window.unwrap_or(200_000);
    let output_budget = agent.llm_config.max_tokens.unwrap_or(4096);

    let max_tool_retries: u32 = 2;
    let mut tool_error_counts: HashMap<String, u32> = HashMap::new();
    let max_result_chars: usize = agent.tool_result_max_chars;
    let skill_timeout = std::time::Duration::from_secs(agent.skill_timeout_secs);

    let mut messages = {
        use orka_core::config::HistoryFilter;
        let raw = ctx.messages().await;
        match (agent.history_filter, agent.history_filter_n) {
            (HistoryFilter::None, _) => Vec::new(),
            (HistoryFilter::LastN, Some(n)) if raw.len() > n => raw[raw.len() - n..].to_vec(),
            _ => raw,
        }
    };
    let (sanitized_messages, removed_tool_results) = sanitize_tool_result_history(messages);
    messages = sanitized_messages;
    if removed_tool_results > 0 {
        warn!(
            agent = %agent.id,
            removed_tool_results,
            "dropped orphaned tool_result blocks before agent execution"
        );
    }
    // Input guardrail: check the trigger text before entering the LLM loop.
    // A Block terminates this node immediately; Modify replaces the trigger.
    if let Some(ref guardrail) = deps.guardrail {
        use orka_core::traits::GuardrailDecision;
        let session = orka_core::Session::new(&ctx.trigger.channel, "");
        match guardrail.check_input(&trigger_text, &session).await {
            Ok(GuardrailDecision::Block(reason)) => {
                warn!(agent = %agent.id, %reason, "input blocked by guardrail");
                return Ok(AgentNodeResult {
                    response: Some(format!("Input blocked: {reason}")),
                    handoff: None,
                    iterations: 0,
                    interrupted: None,
                    attachments: Vec::new(),
                    stop_reason: orka_core::stream::AgentStopReason::Error,
                });
            }
            Ok(GuardrailDecision::Modify(filtered)) => {
                // Swap the last user message with the filtered version
                if let Some(last) = messages.last_mut()
                    && matches!(last.role, orka_llm::client::Role::User)
                {
                    *last = orka_llm::client::ChatMessage::user(filtered);
                }
            }
            _ => {}
        }
    }

    // ── PlanningMode::Always — eager plan generation ──────────────────────────
    // Before the first iteration, generate a structured plan via a dedicated
    // LLM call and inject it into the system prompt.  Skip if a plan is
    // already stored in context (e.g. resumed from checkpoint).
    if agent.planning_mode == PlanningMode::Always
        && ctx.get(&SlotKey::shared(PLAN_SLOT)).await.is_none()
        && let Some(ref llm_client) = deps.llm
    {
        let plan_result: orka_core::Result<Plan> =
            generate_plan(llm_client.as_ref(), &trigger_text, &messages).await;
        match plan_result {
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
    }

    let mut iterations = 0usize;
    let mut tool_turns = 0usize;
    let mut final_response: Option<String> = None;
    let mut handoff: Option<Handoff> = None;
    let mut stop_reason = orka_core::stream::AgentStopReason::Complete;
    // Accumulated media attachments from all skill invocations in this node.
    let mut collected_attachments: Vec<MediaPayload> = Vec::new();
    // Active worktree path for this agent run. Set when `git_worktree_create`
    // succeeds; cleared when `git_worktree_remove` is called.  Propagated into
    // every `SkillContext` so tools operate inside the worktree automatically.
    let mut worktree_cwd: Option<String> = None;

    loop {
        let iteration = iterations;
        iterations += 1;
        let iteration_start = std::time::Instant::now();

        let (sanitized_messages, removed_tool_results) = sanitize_tool_result_history(messages);
        messages = sanitized_messages;
        if removed_tool_results > 0 {
            warn!(
                agent = %agent.id,
                iteration,
                removed_tool_results,
                "dropped orphaned tool_result blocks before llm call"
            );
        }

        // B1: Rebuild tool list from enabled categories each iteration
        if progressive {
            tools.clear();
            tools.extend(synthetic_tools());
            tools.extend(build_skill_tools(&enabled_categories));
            tools.extend(handoff_tools.clone());
            tools.extend(plan_tools.clone());
        }

        // Truncate history to fit context window
        let hint = TokenizerHint::from_model(agent.llm_config.model.as_deref());
        if let HistoryStrategy::RollingWindow { recent_turns } = agent.history_strategy {
            // Keep only the last `recent_turns` conversation turns; summarize
            // the dropped ones incrementally so context is not silently lost.
            let turns = orka_llm::context::group_into_turns(&messages);
            if turns.len() > recent_turns {
                let cutoff = turns.len() - recent_turns;
                let drop_end = turns[cutoff - 1].end;
                let to_drop = messages[..drop_end].to_vec();
                if let Some(ref llm_client) = deps.llm
                    && let Ok(summary) = summarize_messages(llm_client.as_ref(), &to_drop).await
                {
                    ctx.set_conversation_summary(summary).await;
                }
                let dropped = drop_end;
                messages = messages[drop_end..].to_vec();
                warn!(
                    dropped,
                    remaining = messages.len(),
                    recent_turns,
                    "rolling window: trimmed history"
                );
                let history_tokens: u32 = messages
                    .iter()
                    .map(|m| estimate_message_tokens_with_hint(m, hint))
                    .sum();
                deps.stream_registry
                    .send(orka_core::stream::StreamChunk::new(
                        ctx.session_id,
                        envelope.channel.clone(),
                        Some(envelope.id),
                        orka_core::stream::StreamChunkKind::ContextInfo {
                            history_tokens,
                            context_window,
                            messages_truncated: dropped as u32,
                            summary_generated: true,
                        },
                    ));
            }
        } else {
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
                    "truncated history to fit context window"
                );
                let history_tokens: u32 = messages
                    .iter()
                    .map(|m| estimate_message_tokens_with_hint(m, hint))
                    .sum();
                deps.stream_registry
                    .send(orka_core::stream::StreamChunk::new(
                        ctx.session_id,
                        envelope.channel.clone(),
                        Some(envelope.id),
                        orka_core::stream::StreamChunkKind::ContextInfo {
                            history_tokens,
                            context_window,
                            messages_truncated: dropped as u32,
                            summary_generated: false,
                        },
                    ));
            }
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
                stop_reason = orka_core::stream::AgentStopReason::Error;
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
                stop_reason = orka_core::stream::AgentStopReason::Error;
                break;
            }
        };

        if completion.stop_reason == Some(orka_llm::client::StopReason::MaxTokens) {
            warn!("LLM response truncated (max_tokens reached)");
            stop_reason = orka_core::stream::AgentStopReason::MaxTokens;
        }

        let llm_duration_ms = llm_start.elapsed().as_millis() as u64;
        let iteration_tokens =
            u64::from(completion.usage.input_tokens + completion.usage.output_tokens);
        ctx.add_tokens(iteration_tokens);

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

        deps.stream_registry
            .send(orka_core::stream::StreamChunk::new(
                ctx.session_id,
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

        // Parse response — separate thinking, text, and tool calls
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

        // Emit AgentReasoning only when extended thinking produced content
        if !thinking_text.is_empty() {
            deps.event_sink
                .emit(DomainEvent::new(DomainEventKind::AgentReasoning {
                    message_id,
                    iteration,
                    reasoning_text: thinking_text,
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
            messages.push(ChatMessage::assistant(response_text.clone()));
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
            let h = parse_handoff(&agent.id, hc);
            info!(from = %agent.id, to = %h.to, mode = ?h.mode, "agent handoff");
            handoff = Some(h);

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
                messages.push(ChatMessage::new(
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
            messages.push(ChatMessage::new(
                orka_llm::client::Role::Assistant,
                ChatContent::Blocks(blocks),
            ));
        }

        // B1: Intercept synthetic progressive-disclosure tool calls before skill
        // dispatch
        let mut results_map: HashMap<String, (String, bool)> = HashMap::new();
        let mut skill_calls: Vec<&ToolCall> = Vec::new();

        for call in &regular_calls {
            if call.name == "create_plan" || call.name == "update_plan_step" {
                let result: (String, bool) = match call.name.as_str() {
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
                                        let description =
                                            s.get("description")?.as_str()?.to_string();
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
                    "update_plan_step" => {
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

                        // Load current plan from state, update step, save back.
                        let plan_key = SlotKey::shared(PLAN_SLOT);
                        let updated = if let Some(json) = ctx.get(&plan_key).await {
                            match serde_json::from_value::<Plan>(json) {
                                Ok(mut plan) => {
                                    if let Some(step) =
                                        plan.steps.iter_mut().find(|s| s.id == step_id)
                                    {
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
                    other => {
                        tracing::warn!(call_name = %other, "unexpected plan-related tool call — skipped");
                        (format!("Error: unrecognized plan call '{other}'"), true)
                    }
                };
                results_map.insert(call.id.clone(), result);
            } else if progressive
                && (call.name == "list_tool_categories" || call.name == "enable_tools")
            {
                let result = match call.name.as_str() {
                    "list_tool_categories" => {
                        let categories = deps.skills.list_by_category();
                        (
                            serde_json::to_string_pretty(&categories).unwrap_or_default(),
                            false,
                        )
                    }
                    "enable_tools" => {
                        let cat = call
                            .input
                            .get("category")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if cat.is_empty() {
                            ("Error: 'category' parameter is required".to_string(), true)
                        } else {
                            enabled_categories.insert(cat.clone());
                            (format!("Tools in category '{cat}' are now enabled."), false)
                        }
                    }
                    other => {
                        tracing::warn!(call_name = %other, "unexpected progressive tool call — skipped");
                        (
                            format!("Error: unrecognized progressive call '{other}'"),
                            true,
                        )
                    }
                };
                results_map.insert(call.id.clone(), result);
            } else {
                skill_calls.push(call);
            }
        }

        // HITL: if any pending skill call requires human approval, interrupt
        // before running *any* tool in this batch.
        if let Some(call) = skill_calls
            .iter()
            .find(|c| agent.interrupt_before_tools.contains(&c.name))
        {
            let reason = orka_checkpoint::InterruptReason::HumanApproval {
                tool_name: call.name.clone(),
                tool_input: call.input.clone(),
                agent_id: agent.id.to_string(),
            };
            return Ok(AgentNodeResult {
                response: None,
                handoff: None,
                iterations,
                interrupted: Some(reason),
                attachments: Vec::new(),
                stop_reason: orka_core::stream::AgentStopReason::Interrupted,
            });
        }

        let mut join_set = tokio::task::JoinSet::new();

        for call in &skill_calls {
            // Tool-input guardrail: check serialized args before execution.
            // Blocked calls return an error result to the LLM without execution.
            // `modified_input` holds a guardrail-replaced value when `Modify` is returned.
            let mut modified_input: Option<serde_json::Value> = None;
            if let Some(ref guardrail) = deps.guardrail {
                use orka_core::traits::GuardrailDecision;
                let session = orka_core::Session::new(&ctx.trigger.channel, "");
                let input_json = call.input.to_string();
                match guardrail.check_input(&input_json, &session).await {
                    Ok(GuardrailDecision::Block(reason)) => {
                        warn!(skill = %call.name, %reason, "tool input blocked by guardrail");
                        results_map.insert(
                            call.id.clone(),
                            (format!("Tool input blocked by guardrail: {reason}"), true),
                        );
                        continue;
                    }
                    Ok(GuardrailDecision::Modify(modified)) => {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&modified) {
                            modified_input = Some(v);
                        }
                    }
                    _ => {}
                }
            }

            let call_id = call.id.clone();
            let call_name = call.name.clone();
            let call_input = modified_input.unwrap_or_else(|| call.input.clone());
            let skills = deps.skills.clone();
            let event_sink = deps.event_sink.clone();
            let secrets = deps.secrets.clone();
            let skill_max_output_bytes = agent.skill_max_output_bytes;
            let skill_max_duration_ms = agent.skill_max_duration_ms;

            deps.event_sink
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

            let user_cwd = ctx
                .trigger
                .metadata
                .get("workspace:cwd")
                .and_then(|v| v.as_str())
                .map(String::from);
            let worktree_cwd_for_task = worktree_cwd.clone();

            // For coding_delegate on non-custom channels, spawn a progress
            // bridge that forwards significant events to the originating chat.
            let progress_tx_for_spawn = if call_name == "coding_delegate"
                && ctx.trigger.channel != "custom"
            {
                if let Some(ref bus) = deps.bus {
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
                } else {
                    None
                }
            } else {
                None
            };

            join_set.spawn(async move {
                let args: HashMap<String, serde_json::Value> = match call_input {
                    serde_json::Value::Object(map) => map.into_iter().collect(),
                    _ => HashMap::new(),
                };

                let start = std::time::Instant::now();
                let mut skill_ctx = orka_core::SkillContext::new(secrets, Some(event_sink.clone()))
                    .with_user_cwd(user_cwd)
                    .with_worktree_cwd(worktree_cwd_for_task);
                if skill_max_output_bytes.is_some() || skill_max_duration_ms.is_some() {
                    skill_ctx = skill_ctx.with_budget(orka_core::SkillBudget {
                        max_duration_ms: skill_max_duration_ms,
                        max_output_bytes: skill_max_output_bytes,
                    });
                }
                if let Some(tx) = progress_tx_for_spawn {
                    skill_ctx = skill_ctx.with_progress(tx);
                }
                let skill_input = SkillInput::new(args).with_context(skill_ctx);

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
                let task_attachments: Vec<MediaPayload> = match &result {
                    Ok(output) => output.attachments.clone(),
                    Err(_) => Vec::new(),
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

                (call_id, content, is_error, task_attachments)
            });
        }

        while let Some(res) = join_set.join_next().await {
            if let Ok((call_id, content, is_error, attachments)) = res {
                results_map.insert(call_id, (content, is_error));
                collected_attachments.extend(attachments);
            }
        }

        // Update worktree context based on successful worktree skill calls.
        // This enables all subsequent skill calls (coding_delegate, shell_exec,
        // git_*) to automatically operate inside the correct worktree.
        for call in &regular_calls {
            if let Some((content, false)) = results_map.get(&call.id) {
                match call.name.as_str() {
                    "git_worktree_create" => {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(content)
                            && let Some(path) = v.get("path").and_then(|p| p.as_str())
                        {
                            worktree_cwd = Some(path.to_string());
                            debug!(worktree_path = path, "worktree context activated");
                        }
                    }
                    "git_worktree_remove" => {
                        worktree_cwd = None;
                        debug!("worktree context cleared");
                    }
                    _ => {}
                }
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

        if let Some(hint) = build_tool_error_hint(&tool_error_counts, max_tool_retries) {
            result_blocks.push(ContentBlockInput::Text { text: hint });
        }

        messages.push(ChatMessage::new(
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

        tool_turns += 1;
        if tool_turns >= agent.max_turns {
            warn!(max_turns = agent.max_turns, "agent reached max tool turns");
            stop_reason = orka_core::stream::AgentStopReason::MaxTurns;
            break;
        }
    }

    ctx.set_messages(messages).await;

    // Output guardrail: filter the final text response before returning.
    let final_response = match (final_response, &deps.guardrail) {
        (Some(text), Some(guardrail)) => {
            use orka_core::traits::GuardrailDecision;
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
        (resp, _) => resp,
    };

    Ok(AgentNodeResult {
        response: final_response,
        handoff,
        iterations,
        interrupted: None,
        attachments: collected_attachments,
        stop_reason,
    })
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
        .complete_stream_with_tools(&msgs, system, &[], CompletionOptions::default())
        .await
        .map_err(|e| orka_core::Error::Other(e.to_string()))?;

    // Use a no-op stream registry for the summarization call.
    let registry = orka_core::StreamRegistry::new();
    let session_id = orka_core::SessionId::new();
    let completion = consume_stream(stream, &session_id, &registry, "summarize", None)
        .await
        .map_err(|e| orka_core::Error::Other(e.to_string()))?;

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
        .complete_with_options(msgs, system, opts)
        .await
        .map_err(|e| orka_core::Error::Other(e.to_string()))?;

    // Parse response — allow partial structures gracefully
    let raw_plan: RawPlan = serde_json::from_str(&raw)
        .map_err(|e| orka_core::Error::Other(format!("plan parse error: {e} — raw: {raw}")))?;

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
