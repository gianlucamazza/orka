//! Graph execution engine.

use std::{path::Path, sync::Arc, time::Instant};

use orka_checkpoint::{CheckpointStore, RunStatus};
use orka_core::{
    DomainEvent, DomainEventKind, MessageId, OutboundMessage, Payload,
    stream::{StreamChunk, StreamChunkKind},
    traits::{EventSink, Guardrail, MemoryStore, SecretManager},
    types::MediaPayload,
};
use orka_experience::ExperienceService;
use orka_knowledge::FactStore;
use orka_llm::client::LlmClient;
use orka_prompts::template::TemplateRegistry;
use orka_skills::SkillRegistry;
use tracing::{Instrument, info, info_span, warn};

use crate::{
    agent::AgentId,
    context::ExecutionContext,
    graph::{AgentGraph, EdgeCondition, NodeKind},
    handoff::HandoffMode,
    node_runner::run_agent_node,
};

/// Runtime status for Orka's coding delegation layer.
#[derive(Debug, Clone, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct CodingRuntimeStatus {
    /// Whether the public `coding_delegate` tool is currently usable.
    pub tool_available: bool,
    /// Configured default provider selection value.
    pub default_provider: String,
    /// Configured backend selection policy.
    pub selection_policy: String,
    /// Whether Claude Code is available for routing right now.
    pub claude_code_available: bool,
    /// Whether Codex is available for routing right now.
    pub codex_available: bool,
    /// The backend that the router will select for a delegated run, if any.
    pub selected_backend: Option<String>,
    /// Whether the selected backend is configured to modify files.
    pub file_modifications_allowed: bool,
    /// Whether the selected backend is configured to execute commands.
    pub command_execution_allowed: bool,
    /// OS-level allowed filesystem roots from runtime config.
    pub allowed_paths: Vec<String>,
    /// OS-level denied filesystem roots from runtime config.
    pub denied_paths: Vec<String>,
}

impl CodingRuntimeStatus {
    /// Render a deterministic prompt section for runtime tool introspection.
    pub fn render_prompt_section(&self, user_cwd: Option<&str>) -> String {
        let mut available_backends = Vec::new();
        if self.claude_code_available {
            available_backends.push("claude_code");
        }
        if self.codex_available {
            available_backends.push("codex");
        }

        let backend_text = if available_backends.is_empty() {
            "none".to_string()
        } else {
            available_backends.join(", ")
        };

        let availability = if self.tool_available {
            "available"
        } else {
            "unavailable"
        };

        let selected_backend = self.selected_backend.as_deref().unwrap_or("none");
        let file_modifications = if self.file_modifications_allowed {
            "allowed"
        } else {
            "not allowed"
        };
        let command_execution = if self.command_execution_allowed {
            "allowed"
        } else {
            "not allowed"
        };
        let cwd_text = user_cwd.unwrap_or("unknown");
        let cwd_policy = self
            .cwd_policy(user_cwd)
            .unwrap_or_else(|| "unknown".to_string());

        format!(
            "## Coding Runtime\n\n\
`coding_delegate` is currently {availability}.\n\
Configured default provider: `{}`.\n\
Selection policy: `{}`.\n\
Available coding backends in this runtime: {}.\n\
Selected backend for a delegated run right now: `{selected_backend}`.\n\
Selected backend file modifications: {file_modifications}.\n\
Selected backend command execution: {command_execution}.\n\
Current user working directory from the client: `{cwd_text}`.\n\
OS policy for the current working directory: {cwd_policy}.\n\n\
If file modifications are allowed, do not claim that the coding backend lacks write permission unless a delegated run actually failed with a concrete write error.\n\
For questions about Orka coding capabilities, answer from this runtime status instead of reading config files or probing the filesystem.",
            self.default_provider, self.selection_policy, backend_text
        )
    }

    fn cwd_policy(&self, user_cwd: Option<&str>) -> Option<String> {
        let cwd = user_cwd?;
        let path = Path::new(cwd);

        if self
            .denied_paths
            .iter()
            .map(Path::new)
            .any(|denied| path.starts_with(denied))
        {
            return Some("denied by `os.denied_paths`".to_string());
        }

        if self.allowed_paths.is_empty() {
            return Some("allowed (no `os.allowed_paths` restriction configured)".to_string());
        }

        if self
            .allowed_paths
            .iter()
            .map(Path::new)
            .any(|allowed| path.starts_with(allowed))
        {
            Some("allowed by `os.allowed_paths`".to_string())
        } else {
            Some("outside `os.allowed_paths`".to_string())
        }
    }
}

/// External dependencies injected into the executor.
pub struct ExecutorDeps {
    /// Skill registry for resolving and calling tools.
    pub skills: Arc<SkillRegistry>,
    /// Memory store for persisting conversation history.
    pub memory: Arc<dyn MemoryStore>,
    /// Secret manager for resolving credentials.
    pub secrets: Arc<dyn SecretManager>,
    /// Optional LLM client override; falls back to the global default if
    /// `None`.
    pub llm: Option<Arc<dyn LlmClient>>,
    /// Sink for emitting domain events.
    pub event_sink: Arc<dyn EventSink>,
    /// Registry for sending stream chunks to connected sessions.
    pub stream_registry: orka_core::StreamRegistry,
    /// Optional experience service for post-run reflection.
    pub experience: Option<Arc<ExperienceService>>,
    /// Optional semantic fact store for prompt enrichment.
    pub facts: Option<Arc<FactStore>>,
    /// Optional registry of soft (instruction-based) skills.
    pub soft_skills: Option<std::sync::Arc<orka_skills::SoftSkillRegistry>>,
    /// Optional template registry for prompt rendering.
    pub templates: Option<Arc<TemplateRegistry>>,
    /// Runtime status for Orka's coding delegation layer.
    pub coding_runtime: Option<CodingRuntimeStatus>,
    /// Optional guardrail applied to input, tool calls, and output within
    /// every agent node in this graph.
    pub guardrail: Option<Arc<dyn Guardrail>>,
    /// Optional checkpoint store for crash recovery and HITL support.
    ///
    /// When set, the executor writes a checkpoint after every node completes.
    /// A crashed run can be resumed via [`GraphExecutor::resume`] without
    /// reprocessing completed nodes.
    pub checkpoint_store: Option<Arc<dyn CheckpointStore>>,
    /// Message bus used to forward coding-delegate progress events to the
    /// originating chat channel.  When `None`, progress forwarding is
    /// disabled.
    pub bus: Option<Arc<dyn orka_core::traits::MessageBus>>,
}

/// Result of a complete graph execution.
pub struct ExecutionResult {
    /// The final text response from the terminal agent.
    pub response: String,
    /// Media attachments produced by skills during the run.
    /// Emitted as separate outbound messages after the text response.
    pub attachments: Vec<MediaPayload>,
    /// Total agents that executed.
    pub agents_executed: Vec<String>,
    /// Total LLM iterations.
    pub total_iterations: usize,
    /// Total tokens consumed.
    pub total_tokens: u64,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Why the agent stopped executing.
    pub stop_reason: orka_core::stream::AgentStopReason,
}

impl ExecutionResult {
    /// Convert the result into `OutboundMessage`s for the worker.
    ///
    /// Produces one text message (if there is a response) followed by one
    /// additional message per media attachment generated during execution.
    pub fn into_outbound_messages(self, ctx: &ExecutionContext) -> Vec<OutboundMessage> {
        let mut out: Vec<OutboundMessage> = Vec::new();
        let assistant_message_id = MessageId::new();

        if !self.response.is_empty() {
            let mut msg = OutboundMessage::text(
                ctx.trigger.channel.clone(),
                ctx.session_id,
                self.response,
                Some(ctx.trigger.id),
            );
            msg.metadata.clone_from(&ctx.trigger.metadata);
            msg.metadata
                .entry("source_channel".into())
                .or_insert_with(|| serde_json::Value::String(ctx.trigger.channel.clone()));
            msg.metadata.insert(
                "assistant_message_id".into(),
                serde_json::json!(assistant_message_id.to_string()),
            );
            if self.stop_reason != orka_core::stream::AgentStopReason::Complete {
                msg.metadata.insert(
                    "stop_reason".into(),
                    serde_json::to_value(self.stop_reason).unwrap_or_default(),
                );
            }
            out.push(msg);
        }

        for attachment in self.attachments {
            let mut media_msg = OutboundMessage::new(
                ctx.trigger.channel.clone(),
                ctx.session_id,
                Payload::Media(attachment),
                Some(ctx.trigger.id),
            );
            media_msg.metadata.clone_from(&ctx.trigger.metadata);
            media_msg
                .metadata
                .entry("source_channel".into())
                .or_insert_with(|| serde_json::Value::String(ctx.trigger.channel.clone()));
            media_msg.metadata.insert(
                "assistant_message_id".into(),
                serde_json::json!(assistant_message_id.to_string()),
            );
            out.push(media_msg);
        }

        out
    }
}

/// Snapshot of mutable execution state passed to
/// [`GraphExecutor::maybe_save_checkpoint`].
///
/// Bundles the arguments needed for checkpoint creation to stay within the
/// 7-argument clippy limit while keeping the public API clean.
struct CheckpointSnap<'a> {
    completed_node: &'a AgentId,
    resume_node: Option<&'a AgentId>,
    total_iterations: usize,
    agents_executed: &'a [String],
    status: RunStatus,
}

/// Executes an `AgentGraph` driven by an `ExecutionContext`.
pub struct GraphExecutor {
    /// Shared external dependencies used during execution.
    pub deps: Arc<ExecutorDeps>,
}

impl GraphExecutor {
    /// Create a new executor wrapping the given dependencies.
    pub fn new(deps: ExecutorDeps) -> Self {
        Self {
            deps: Arc::new(deps),
        }
    }

    /// Execute the graph and return the final result.
    pub async fn execute(
        &self,
        graph: &AgentGraph,
        ctx: &ExecutionContext,
    ) -> orka_core::Result<ExecutionResult> {
        let span = info_span!(
            "graph.execute",
            run_id = %ctx.run_id,
            graph_id = %graph.id,
            session_id = %ctx.session_id,
        );

        // Register graph-level reducer strategies on the context so concurrent
        // fan-out writes merge deterministically.
        if !graph.reducers.is_empty() {
            ctx.set_reducers(graph.reducers.clone()).await;
        }

        let result = self
            .execute_inner(graph, ctx, None)
            .instrument(span)
            .await?;

        // Emit GraphCompleted event
        self.deps
            .event_sink
            .emit(DomainEvent::new(DomainEventKind::GraphCompleted {
                run_id: ctx.run_id.to_string(),
                graph_id: graph.id.clone(),
                agents_executed: result.agents_executed.clone(),
                total_iterations: result.total_iterations,
                total_tokens: result.total_tokens,
                duration_ms: result.duration_ms,
            }))
            .await;

        Ok(result)
    }

    /// Resume a previously interrupted or crashed graph run from its latest
    /// checkpoint.
    ///
    /// Loads the checkpoint from the configured store, reconstructs the
    /// `ExecutionContext`, and continues execution from the node recorded in
    /// `Checkpoint::resume_node`.
    ///
    /// Returns `Ok(None)` when no checkpoint exists for `run_id` or when the
    /// run has already reached a terminal state (no `resume_node`).
    pub async fn resume(
        &self,
        run_id: &str,
        graph: &AgentGraph,
    ) -> orka_core::Result<Option<ExecutionResult>> {
        let Some(store) = &self.deps.checkpoint_store else {
            return Err(orka_core::Error::Other(
                "resume requires a checkpoint_store".into(),
            ));
        };

        let Some(checkpoint) = store.load_latest(run_id).await? else {
            return Ok(None);
        };

        let Some(ref resume_node_id) = checkpoint.resume_node else {
            // The run completed or failed terminally — nothing to resume.
            return Ok(None);
        };

        let starting_node = AgentId::new(resume_node_id.as_str());
        let ctx = ExecutionContext::from_checkpoint(&checkpoint);

        // Restore reducer strategies so fan-out writes merge correctly on resume.
        if !graph.reducers.is_empty() {
            ctx.set_reducers(graph.reducers.clone()).await;
        }

        let span = info_span!(
            "graph.resume",
            run_id = %ctx.run_id,
            graph_id = %graph.id,
            session_id = %ctx.session_id,
            resume_from = %resume_node_id,
        );

        let result = self
            .execute_inner(graph, &ctx, Some(starting_node))
            .instrument(span)
            .await?;

        self.deps
            .event_sink
            .emit(DomainEvent::new(DomainEventKind::GraphCompleted {
                run_id: ctx.run_id.to_string(),
                graph_id: graph.id.clone(),
                agents_executed: result.agents_executed.clone(),
                total_iterations: result.total_iterations,
                total_tokens: result.total_tokens,
                duration_ms: result.duration_ms,
            }))
            .await;

        Ok(Some(result))
    }

    /// Save a checkpoint if a checkpoint store is configured.
    ///
    /// Checkpoint failures are logged as warnings but never propagate — a
    /// failed checkpoint write must not abort the ongoing graph execution.
    async fn maybe_save_checkpoint(
        &self,
        ctx: &ExecutionContext,
        graph: &AgentGraph,
        snap: CheckpointSnap<'_>,
    ) {
        let Some(store) = &self.deps.checkpoint_store else {
            return;
        };

        let ckpt = ctx
            .to_checkpoint(
                &graph.id,
                snap.completed_node.as_str(),
                snap.resume_node.map(super::agent::AgentId::as_str),
                snap.total_iterations,
                snap.agents_executed.to_vec(),
                snap.status,
            )
            .await;

        if let Err(e) = store.save(&ckpt).await {
            warn!(
                run_id = %ctx.run_id,
                checkpoint_id = %ckpt.id,
                error = %e,
                "checkpoint.save failed — execution continues"
            );
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn execute_inner(
        &self,
        graph: &AgentGraph,
        ctx: &ExecutionContext,
        starting_node: Option<AgentId>,
    ) -> orka_core::Result<ExecutionResult> {
        let policy = &graph.termination;
        let max_duration = policy.max_duration;
        let start = Instant::now();

        let mut current_id = starting_node.unwrap_or_else(|| graph.entry.clone());
        let mut total_iterations = 0usize;
        let mut agents_executed: Vec<String> = Vec::new();
        let mut final_response = String::new();
        let mut all_attachments: Vec<MediaPayload> = Vec::new();
        let mut global_iteration = 0usize;
        let mut final_stop_reason = orka_core::stream::AgentStopReason::Complete;

        // Add initial user message to context if not already present
        let trigger_text = match &ctx.trigger.payload {
            Payload::Text(t) => t.clone(),
            Payload::RichInput(input) => input.text.clone().unwrap_or_default(),
            Payload::Command(cmd) => format!("/{}", cmd.name),
            _ => String::new(),
        };
        if !trigger_text.is_empty() && ctx.messages().await.is_empty() {
            ctx.push_message(orka_llm::client::ChatMessage::user(trigger_text.clone()))
                .await;
        }

        loop {
            global_iteration += 1;

            // Check termination policy
            if global_iteration > policy.max_total_iterations {
                warn!(
                    max = policy.max_total_iterations,
                    "graph max total iterations exceeded"
                );
                break;
            }
            if start.elapsed() > max_duration {
                warn!("graph max duration exceeded");
                break;
            }
            if let Some(max_tokens) = policy.max_total_tokens
                && ctx.total_tokens() > max_tokens
            {
                warn!(max_tokens, "graph max token budget exceeded");
                break;
            }

            let Some(node) = graph.get_node(&current_id) else {
                warn!(agent_id = %current_id, "graph node not found");
                break;
            };

            agents_executed.push(current_id.to_string());

            let agent_span = info_span!(
                "agent.execute",
                agent_id = %node.agent.id,
                run_id = %ctx.run_id,
            );

            match &node.kind {
                NodeKind::Agent => {
                    // Emit AgentSwitch so adapters can show which agent is responding
                    {
                        self.deps.stream_registry.send(StreamChunk::new(
                            ctx.session_id,
                            ctx.trigger.channel.clone(),
                            Some(ctx.trigger.id),
                            StreamChunkKind::AgentSwitch {
                                agent_id: node.agent.id.to_string(),
                                display_name: node.agent.display_name.clone(),
                            },
                        ));
                    }
                    let result = run_agent_node(&node.agent, ctx, &self.deps, graph)
                        .instrument(agent_span)
                        .await?;

                    total_iterations += result.iterations;
                    all_attachments.extend(result.attachments.iter().cloned());

                    // Emit AgentCompleted event
                    self.deps
                        .event_sink
                        .emit(DomainEvent::new(DomainEventKind::AgentCompleted {
                            run_id: ctx.run_id.to_string(),
                            agent_id: node.agent.id.to_string(),
                            iterations: result.iterations,
                            tokens: ctx.total_tokens(),
                            duration_ms: start.elapsed().as_millis() as u64,
                            success: result.response.is_some() || result.handoff.is_some(),
                        }))
                        .await;

                    // HITL: save interrupted checkpoint and stop execution.
                    if let Some(interrupt_reason) = result.interrupted {
                        let tool_name = match &interrupt_reason {
                            orka_checkpoint::InterruptReason::HumanApproval {
                                tool_name, ..
                            } => tool_name.clone(),
                            orka_checkpoint::InterruptReason::Breakpoint { node_id } => {
                                node_id.clone()
                            }
                            _ => String::new(),
                        };
                        self.deps
                            .event_sink
                            .emit(DomainEvent::new(DomainEventKind::RunInterrupted {
                                run_id: ctx.run_id.to_string(),
                                agent_id: node.agent.id.to_string(),
                                tool_name,
                            }))
                            .await;
                        self.maybe_save_checkpoint(
                            ctx,
                            graph,
                            CheckpointSnap {
                                completed_node: &node.agent.id,
                                // Resume from the same node so the tool is
                                // re-dispatched after approval.
                                resume_node: Some(&current_id),
                                total_iterations,
                                agents_executed: &agents_executed,
                                status: RunStatus::Interrupted {
                                    reason: interrupt_reason,
                                },
                            },
                        )
                        .await;
                        return Ok(ExecutionResult {
                            response: String::new(),
                            attachments: Vec::new(),
                            agents_executed,
                            total_iterations,
                            total_tokens: ctx.total_tokens(),
                            duration_ms: start.elapsed().as_millis() as u64,
                            stop_reason: orka_core::stream::AgentStopReason::Interrupted,
                        });
                    }

                    if let Some(handoff) = result.handoff {
                        // Emit AgentDelegated event
                        self.deps
                            .event_sink
                            .emit(DomainEvent::new(DomainEventKind::AgentDelegated {
                                run_id: ctx.run_id.to_string(),
                                source_agent: handoff.from.to_string(),
                                target_agent: handoff.to.to_string(),
                                mode: format!("{:?}", handoff.mode),
                                reason: handoff.reason.clone(),
                            }))
                            .await;

                        match handoff.mode {
                            HandoffMode::Transfer => {
                                info!(
                                    from = %handoff.from,
                                    to = %handoff.to,
                                    "agent transfer"
                                );
                                // Inject structured context provided by the
                                // source agent so the target has visibility
                                if !handoff.context_transfer.is_empty()
                                    && let Ok(json) =
                                        serde_json::to_string_pretty(&handoff.context_transfer)
                                {
                                    ctx.push_message(orka_llm::client::ChatMessage::user(format!(
                                        "[Handoff context from {}]: {json}",
                                        handoff.from
                                    )))
                                    .await;
                                }
                                let next = handoff.to.clone();
                                self.maybe_save_checkpoint(
                                    ctx,
                                    graph,
                                    CheckpointSnap {
                                        completed_node: &node.agent.id,
                                        resume_node: Some(&next),
                                        total_iterations,
                                        agents_executed: &agents_executed,
                                        status: RunStatus::Running,
                                    },
                                )
                                .await;
                                current_id = next;
                                continue;
                            }
                            HandoffMode::Delegate => {
                                // Execute target, then return to source
                                info!(
                                    from = %handoff.from,
                                    to = %handoff.to,
                                    "agent delegate"
                                );
                                if !handoff.context_transfer.is_empty()
                                    && let Ok(json) =
                                        serde_json::to_string_pretty(&handoff.context_transfer)
                                {
                                    ctx.push_message(orka_llm::client::ChatMessage::user(format!(
                                        "[Handoff context from {}]: {json}",
                                        handoff.from
                                    )))
                                    .await;
                                }
                                let Some(target_node) = graph.get_node(&handoff.to) else {
                                    warn!(to = %handoff.to, "delegate target not found");
                                    current_id = graph.entry.clone();
                                    continue;
                                };
                                {
                                    self.deps.stream_registry.send(StreamChunk::new(
                                        ctx.session_id,
                                        ctx.trigger.channel.clone(),
                                        Some(ctx.trigger.id),
                                        StreamChunkKind::AgentSwitch {
                                            agent_id: target_node.agent.id.to_string(),
                                            display_name: target_node.agent.display_name.clone(),
                                        },
                                    ));
                                }
                                let delegate_result =
                                    run_agent_node(&target_node.agent, ctx, &self.deps, graph)
                                        .await?;
                                total_iterations += delegate_result.iterations;
                                all_attachments.extend(delegate_result.attachments.iter().cloned());
                                if delegate_result.stop_reason
                                    != orka_core::stream::AgentStopReason::Complete
                                {
                                    final_stop_reason = delegate_result.stop_reason;
                                    if let Some(resp) = delegate_result.response {
                                        final_response = resp;
                                    }
                                    warn!(
                                        agent_id = %target_node.agent.id,
                                        parent_agent_id = %node.agent.id,
                                        stop_reason = ?delegate_result.stop_reason,
                                        "delegate agent stopped with non-complete reason; graph execution terminated"
                                    );
                                    break;
                                }

                                if let Some(resp) = delegate_result.response {
                                    // Feed the delegate result back as a tool result message
                                    ctx.push_message(orka_llm::client::ChatMessage::user(format!(
                                        "[Delegate result from {}]: {resp}",
                                        handoff.to
                                    )))
                                    .await;
                                }

                                // Source agent continues
                                continue;
                            }
                        }
                    }

                    final_stop_reason = result.stop_reason;
                    if result.stop_reason != orka_core::stream::AgentStopReason::Complete {
                        if let Some(resp) = result.response {
                            final_response = resp;
                        }
                        warn!(
                            agent_id = %node.agent.id,
                            stop_reason = ?result.stop_reason,
                            "agent stopped with non-complete reason; graph execution terminated"
                        );
                        self.maybe_save_checkpoint(
                            ctx,
                            graph,
                            CheckpointSnap {
                                completed_node: &node.agent.id,
                                resume_node: None,
                                total_iterations,
                                agents_executed: &agents_executed,
                                status: checkpoint_status_for_stop_reason(result.stop_reason),
                            },
                        )
                        .await;
                        break;
                    }
                    if let Some(resp) = result.response {
                        final_response = resp.clone();

                        // Check if this is a terminal agent
                        if policy.terminal_agents.contains(&current_id)
                            || graph.outgoing_edges(&current_id).is_empty()
                        {
                            self.maybe_save_checkpoint(
                                ctx,
                                graph,
                                CheckpointSnap {
                                    completed_node: &node.agent.id,
                                    resume_node: None,
                                    total_iterations,
                                    agents_executed: &agents_executed,
                                    status: RunStatus::Completed,
                                },
                            )
                            .await;
                            break;
                        }

                        // Evaluate outgoing edges
                        let next = self.evaluate_edges(graph, &current_id, &resp, ctx).await;
                        if let Some(next_id) = next {
                            self.maybe_save_checkpoint(
                                ctx,
                                graph,
                                CheckpointSnap {
                                    completed_node: &node.agent.id,
                                    resume_node: Some(&next_id),
                                    total_iterations,
                                    agents_executed: &agents_executed,
                                    status: RunStatus::Running,
                                },
                            )
                            .await;
                            current_id = next_id;
                            continue;
                        }
                        // No matching edge — terminal
                        self.maybe_save_checkpoint(
                            ctx,
                            graph,
                            CheckpointSnap {
                                completed_node: &node.agent.id,
                                resume_node: None,
                                total_iterations,
                                agents_executed: &agents_executed,
                                status: RunStatus::Completed,
                            },
                        )
                        .await;
                        break;
                    }
                }

                NodeKind::Router => {
                    // Router: evaluate edges without LLM, pick first matching
                    let edges = graph.outgoing_edges(&current_id);
                    let mut routed = false;
                    for edge in edges {
                        if self.edge_matches(edge, "", ctx).await {
                            let next_id = edge.target.clone();
                            self.maybe_save_checkpoint(
                                ctx,
                                graph,
                                CheckpointSnap {
                                    completed_node: &current_id,
                                    resume_node: Some(&next_id),
                                    total_iterations,
                                    agents_executed: &agents_executed,
                                    status: RunStatus::Running,
                                },
                            )
                            .await;
                            current_id = next_id;
                            routed = true;
                            break;
                        }
                    }
                    if !routed {
                        break;
                    }
                }

                NodeKind::FanOut { max_concurrency } => {
                    // Fan-out: dispatch Agent/Router/FanOut successors in parallel.
                    // Any FanIn-kind successor is the continuation node reached after
                    // all parallel branches complete.
                    let edges = graph.outgoing_edges(&current_id);
                    let semaphore = max_concurrency
                        .map(|n| std::sync::Arc::new(tokio::sync::Semaphore::new(n)));
                    let mut join_set = tokio::task::JoinSet::new();
                    let mut fan_in_id: Option<AgentId> = None;

                    for edge in edges {
                        let target_id = edge.target.clone();
                        let target_node = match graph.get_node(&target_id) {
                            Some(n) => n.clone(),
                            None => continue,
                        };
                        // Route to FanIn after all parallel branches finish.
                        if matches!(target_node.kind, NodeKind::FanIn) {
                            fan_in_id = Some(target_id);
                            continue;
                        }
                        let ctx = ctx.clone();
                        let deps = self.deps.clone();
                        let graph_clone = graph.clone();
                        let sem = semaphore.clone();

                        join_set.spawn(async move {
                            // Acquire a concurrency permit if a limit is set.
                            // Dropping `_permit` at the end of this block releases
                            // the slot for the next waiting branch.
                            let _permit = if let Some(s) = &sem {
                                #[allow(clippy::expect_used)]
                                Some(s.acquire().await.expect("fan-out semaphore closed"))
                            } else {
                                None
                            };
                            {
                                deps.stream_registry.send(StreamChunk::new(
                                    ctx.session_id,
                                    ctx.trigger.channel.clone(),
                                    Some(ctx.trigger.id),
                                    StreamChunkKind::AgentSwitch {
                                        agent_id: target_node.agent.id.to_string(),
                                        display_name: target_node.agent.display_name.clone(),
                                    },
                                ));
                            }
                            run_agent_node(&target_node.agent, &ctx, &deps, &graph_clone).await
                        });
                    }

                    while let Some(res) = join_set.join_next().await {
                        match res {
                            Ok(Ok(node_result)) => {
                                total_iterations += node_result.iterations;
                                all_attachments.extend(node_result.attachments.iter().cloned());
                                if node_result.stop_reason
                                    != orka_core::stream::AgentStopReason::Complete
                                {
                                    final_stop_reason = node_result.stop_reason;
                                    if let Some(resp) = node_result.response {
                                        final_response = resp;
                                    }
                                    warn!(
                                        fan_out_node = %current_id,
                                        stop_reason = ?node_result.stop_reason,
                                        "fan-out branch stopped with non-complete reason; graph execution terminated"
                                    );
                                    join_set.abort_all();
                                    break;
                                }
                                if let Some(resp) = node_result.response {
                                    final_response = resp;
                                }
                            }
                            Ok(Err(e)) => {
                                final_stop_reason = orka_core::stream::AgentStopReason::Error;
                                final_response = format!("Fan-out agent failed: {e}");
                                warn!(%e, fan_out_node = %current_id, "fan-out agent failed");
                                join_set.abort_all();
                                break;
                            }
                            Err(e) => {
                                final_stop_reason = orka_core::stream::AgentStopReason::Error;
                                final_response = format!("Fan-out task panicked: {e}");
                                warn!(%e, fan_out_node = %current_id, "fan-out task panicked");
                                join_set.abort_all();
                                break;
                            }
                        }
                    }

                    if final_stop_reason != orka_core::stream::AgentStopReason::Complete {
                        break;
                    }

                    // Continue to FanIn node if present, otherwise terminate.
                    match fan_in_id {
                        Some(next_id) => {
                            current_id = next_id;
                        }
                        None => break,
                    }
                }

                NodeKind::FanIn => {
                    // FanIn: synthesise parallel results, then continue graph traversal.
                    {
                        self.deps.stream_registry.send(StreamChunk::new(
                            ctx.session_id,
                            ctx.trigger.channel.clone(),
                            Some(ctx.trigger.id),
                            StreamChunkKind::AgentSwitch {
                                agent_id: node.agent.id.to_string(),
                                display_name: node.agent.display_name.clone(),
                            },
                        ));
                    }
                    let result = run_agent_node(&node.agent, ctx, &self.deps, graph)
                        .instrument(agent_span)
                        .await?;
                    total_iterations += result.iterations;
                    all_attachments.extend(result.attachments.iter().cloned());
                    final_stop_reason = result.stop_reason;
                    if result.stop_reason != orka_core::stream::AgentStopReason::Complete {
                        if let Some(resp) = result.response {
                            final_response = resp;
                        }
                        warn!(
                            agent_id = %node.agent.id,
                            stop_reason = ?result.stop_reason,
                            "fan-in node stopped with non-complete reason; graph execution terminated"
                        );
                        self.maybe_save_checkpoint(
                            ctx,
                            graph,
                            CheckpointSnap {
                                completed_node: &node.agent.id,
                                resume_node: None,
                                total_iterations,
                                agents_executed: &agents_executed,
                                status: checkpoint_status_for_stop_reason(result.stop_reason),
                            },
                        )
                        .await;
                        break;
                    }
                    if let Some(resp) = result.response {
                        final_response = resp.clone();
                        // Evaluate outgoing edges — FanIn can feed into further nodes.
                        let next = self.evaluate_edges(graph, &current_id, &resp, ctx).await;
                        match next {
                            Some(next_id) => {
                                current_id = next_id;
                                continue;
                            }
                            None => break,
                        }
                    }
                    break;
                }
            }
        }

        // Emit stream Done chunk
        self.deps.stream_registry.send(StreamChunk::new(
            ctx.session_id,
            ctx.trigger.channel.clone(),
            Some(ctx.trigger.id),
            StreamChunkKind::Done,
        ));

        Ok(ExecutionResult {
            response: final_response,
            attachments: all_attachments,
            agents_executed,
            total_iterations,
            total_tokens: ctx.total_tokens(),
            duration_ms: start.elapsed().as_millis() as u64,
            stop_reason: final_stop_reason,
        })
    }

    async fn evaluate_edges(
        &self,
        graph: &AgentGraph,
        from: &AgentId,
        output: &str,
        ctx: &ExecutionContext,
    ) -> Option<AgentId> {
        let edges = graph.outgoing_edges(from);
        for edge in edges {
            if self.edge_matches(edge, output, ctx).await {
                return Some(edge.target.clone());
            }
        }
        None
    }

    async fn edge_matches(
        &self,
        edge: &crate::graph::Edge,
        output: &str,
        ctx: &ExecutionContext,
    ) -> bool {
        match &edge.condition {
            None | Some(EdgeCondition::Always) => true,
            Some(EdgeCondition::OutputContains(pattern)) => output.contains(pattern.as_str()),
            Some(EdgeCondition::StateMatch { key, pattern }) => {
                ctx.get(key).await.as_ref() == Some(pattern)
            }
        }
    }
}

fn checkpoint_status_for_stop_reason(stop_reason: orka_core::stream::AgentStopReason) -> RunStatus {
    match stop_reason {
        orka_core::stream::AgentStopReason::Error => RunStatus::Failed {
            error: "agent execution terminated with error".to_string(),
        },
        orka_core::stream::AgentStopReason::Complete
        | orka_core::stream::AgentStopReason::MaxTurns
        | orka_core::stream::AgentStopReason::MaxTokens
        | orka_core::stream::AgentStopReason::Interrupted => RunStatus::Completed,
    }
}
