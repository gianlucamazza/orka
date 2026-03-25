//! Single-agent LLM tool loop, extracted from `workspace_handler.rs`.

use std::{collections::HashMap, sync::Arc};

use orka_core::{DomainEvent, DomainEventKind, SkillInput, truncate_tool_result};
use orka_llm::{
    client::{
        ChatContent, ChatMessage, CompletionOptions, ContentBlock, ContentBlockInput, ToolCall,
        ToolDefinition,
    },
    consume_stream,
    context::{
        TokenizerHint, available_history_budget_with_hint, estimate_message_tokens_with_hint,
        truncate_history_with_hint,
    },
    infer_provider,
};
use orka_prompts::pipeline::PipelineConfig;
use tracing::{Instrument, debug, info, info_span, warn};

use crate::{
    agent::Agent,
    context::ExecutionContext,
    executor::ExecutorDeps,
    graph::AgentGraph,
    handoff::{Handoff, HandoffMode},
    tools::build_handoff_tools,
};

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
                ToolDefinition::new(
                    skill.name(),
                    skill.description(),
                    skill.schema().parameters.clone(),
                )
            })
            .collect()
    };

    let handoff_tools = build_handoff_tools(agent, graph);

    let initial_skill_tools = build_skill_tools(&enabled_categories);
    let mut tools: Vec<ToolDefinition> = if progressive {
        let mut t = synthetic_tools();
        t.extend(initial_skill_tools);
        t.extend(handoff_tools.clone());
        t
    } else {
        let mut t = initial_skill_tools;
        t.extend(handoff_tools.clone());
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
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_else(|| vec![workspace_name.to_string()]);
    let cwd = ctx
        .trigger
        .metadata
        .get("workspace:cwd")
        .and_then(|v| v.as_str())
        .map(String::from);

    // Retrieve principles if experience is available
    let principles = if let Some(ref exp) = deps.experience {
        match exp
            .retrieve_principles(&trigger_text, agent.id.0.as_ref())
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

    // Get soft skill section
    let soft_skill_section = if let Some(ref soft_reg) = deps.soft_skills {
        if !soft_reg.is_empty() {
            let selected_names: Vec<&str> =
                if soft_reg.selection_mode == orka_skills::SoftSkillSelectionMode::Keyword {
                    soft_reg.filter_by_message(&trigger_text)
                } else {
                    soft_reg.list()
                };
            soft_reg.build_prompt_section(&selected_names)
        } else {
            String::new()
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

    // Build system prompt using context providers and pipeline
    let system_prompt = {
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
    let max_result_chars: usize = 50_000;
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

    let mut iterations = 0usize;
    let mut final_response: Option<String> = None;
    let mut handoff: Option<Handoff> = None;

    for iteration in 0..agent.max_iterations {
        iterations = iteration + 1;
        let iteration_start = std::time::Instant::now();

        // B1: Rebuild tool list from enabled categories each iteration
        if progressive {
            tools.clear();
            tools.extend(synthetic_tools());
            tools.extend(build_skill_tools(&enabled_categories));
            tools.extend(handoff_tools.clone());
        }

        // Truncate history to fit context window
        let hint = TokenizerHint::from_model(agent.llm_config.model.as_deref());
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
            if progressive && (call.name == "list_tool_categories" || call.name == "enable_tools") {
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
                    _ => unreachable!(),
                };
                results_map.insert(call.id.clone(), result);
            } else {
                skill_calls.push(call);
            }
        }

        let mut join_set = tokio::task::JoinSet::new();

        for call in &skill_calls {
            // Tool-input guardrail: check serialized args before execution.
            // Blocked calls return an error result to the LLM without execution.
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
                        // Use modified args — the call is reconstructed below via call_input
                        // We fall through; the override will be handled in the spawn
                        let _ = modified; // TODO: thread modified args through spawn
                    }
                    _ => {}
                }
            }

            let call_id = call.id.clone();
            let call_name = call.name.clone();
            let call_input = call.input.clone();
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

            join_set.spawn(async move {
                let args: HashMap<String, serde_json::Value> = match call_input {
                    serde_json::Value::Object(map) => map.into_iter().collect(),
                    _ => HashMap::new(),
                };

                let start = std::time::Instant::now();
                let mut skill_ctx = orka_core::SkillContext::new(secrets, Some(event_sink.clone()))
                    .with_user_cwd(user_cwd);
                if skill_max_output_bytes.is_some() || skill_max_duration_ms.is_some() {
                    skill_ctx = skill_ctx.with_budget(orka_core::SkillBudget {
                        max_duration_ms: skill_max_duration_ms,
                        max_output_bytes: skill_max_output_bytes,
                    });
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

                (call_id, content, is_error)
            });
        }

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

        if iteration == agent.max_iterations - 1 {
            warn!(
                max_iterations = agent.max_iterations,
                "agent reached max iterations"
            );
            final_response = Some("I reached the maximum number of reasoning steps.".to_string());
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
            soft_skills: None,
            templates: None,
            coding_runtime: None,
            guardrail: None,
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
        assert!(hint.contains("2"));
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
            soft_skills: None,
            templates: None,
            coding_runtime: None,
            guardrail: None,
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
    async fn max_iterations_cap() {
        // Queue enough tool-call responses to saturate max_iterations.
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
        agent.max_iterations = 3;

        let ctx = minimal_ctx();
        let deps = minimal_deps(mock);
        let graph = minimal_graph(&agent.id);

        let result = run_agent_node(&agent, &ctx, &deps, &graph).await.unwrap();

        assert_eq!(result.iterations, 3);
        let response = result.response.unwrap_or_default();
        assert!(
            response.contains("maximum"),
            "expected max-iterations message, got: {response}"
        );
    }
}
