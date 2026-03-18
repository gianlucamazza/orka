//! Graph execution engine.

use std::sync::Arc;
use std::time::Instant;

use orka_core::traits::{EventSink, MemoryStore, SecretManager};
use orka_core::{DomainEvent, DomainEventKind, OutboundMessage, Payload};
use orka_experience::ExperienceService;
use orka_llm::client::LlmClient;
use orka_skills::SkillRegistry;
use tracing::{Instrument, info, info_span, warn};

use crate::agent::AgentId;
use crate::context::ExecutionContext;
use crate::graph::{AgentGraph, EdgeCondition, NodeKind};
use crate::handoff::HandoffMode;
use crate::node_runner::run_agent_node;

/// External dependencies injected into the executor.
pub struct ExecutorDeps {
    pub skills: Arc<SkillRegistry>,
    pub memory: Arc<dyn MemoryStore>,
    pub secrets: Arc<dyn SecretManager>,
    pub llm: Option<Arc<dyn LlmClient>>,
    pub event_sink: Arc<dyn EventSink>,
    pub stream_registry: orka_core::StreamRegistry,
    pub experience: Option<Arc<ExperienceService>>,
}

/// Result of a complete graph execution.
pub struct ExecutionResult {
    /// The final text response from the terminal agent.
    pub response: String,
    /// Total agents that executed.
    pub agents_executed: Vec<String>,
    /// Total LLM iterations.
    pub total_iterations: usize,
    /// Total tokens consumed.
    pub total_tokens: u64,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
}

impl ExecutionResult {
    /// Convert the result into `OutboundMessage`s for the worker.
    pub fn into_outbound_messages(self, ctx: &ExecutionContext) -> Vec<OutboundMessage> {
        if self.response.is_empty() {
            return Vec::new();
        }
        let mut msg = OutboundMessage::text(
            ctx.trigger.channel.clone(),
            ctx.session_id,
            self.response,
            Some(ctx.trigger.id),
        );
        msg.metadata = ctx.trigger.metadata.clone();
        vec![msg]
    }
}

/// Executes an `AgentGraph` driven by an `ExecutionContext`.
pub struct GraphExecutor {
    pub deps: Arc<ExecutorDeps>,
}

impl GraphExecutor {
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

        let result = self.execute_inner(graph, ctx).instrument(span).await?;

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

    async fn execute_inner(
        &self,
        graph: &AgentGraph,
        ctx: &ExecutionContext,
    ) -> orka_core::Result<ExecutionResult> {
        let policy = &graph.termination;
        let max_duration = policy.max_duration;
        let start = Instant::now();

        let mut current_id = graph.entry.clone();
        let mut total_iterations = 0usize;
        let mut agents_executed: Vec<String> = Vec::new();
        let mut final_response = String::new();
        let mut global_iteration = 0usize;

        // Add initial user message to context if not already present
        let trigger_text = match &ctx.trigger.payload {
            Payload::Text(t) => t.clone(),
            Payload::Command(cmd) => format!("/{}", cmd.name),
            _ => String::new(),
        };
        if !trigger_text.is_empty() && ctx.messages().await.is_empty() {
            ctx.push_message(orka_llm::client::ChatMessageExt::user(trigger_text.clone()))
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
            if let Some(max_tokens) = policy.max_total_tokens {
                if ctx.total_tokens() > max_tokens {
                    warn!(max_tokens, "graph max token budget exceeded");
                    break;
                }
            }

            let node = match graph.get_node(&current_id) {
                Some(n) => n,
                None => {
                    warn!(agent_id = %current_id, "graph node not found");
                    break;
                }
            };

            agents_executed.push(current_id.0.to_string());

            let agent_span = info_span!(
                "agent.execute",
                agent_id = %node.agent.id,
                run_id = %ctx.run_id,
            );

            match &node.kind {
                NodeKind::Agent => {
                    // Emit AgentSwitch so adapters can show which agent is responding
                    {
                        use orka_core::stream::{StreamChunk, StreamChunkKind};
                        self.deps.stream_registry.send(StreamChunk::new(
                            ctx.session_id,
                            ctx.trigger.channel.clone(),
                            Some(ctx.trigger.id),
                            StreamChunkKind::AgentSwitch {
                                agent_id: node.agent.id.0.to_string(),
                                display_name: node.agent.display_name.clone(),
                            },
                        ));
                    }
                    let result = run_agent_node(&node.agent, ctx, &self.deps, graph)
                        .instrument(agent_span)
                        .await?;

                    total_iterations += result.iterations;

                    // Emit AgentCompleted event
                    self.deps
                        .event_sink
                        .emit(DomainEvent::new(DomainEventKind::AgentCompleted {
                            run_id: ctx.run_id.to_string(),
                            agent_id: node.agent.id.0.to_string(),
                            iterations: result.iterations,
                            tokens: ctx.total_tokens(),
                            duration_ms: start.elapsed().as_millis() as u64,
                            success: result.response.is_some() || result.handoff.is_some(),
                        }))
                        .await;

                    if let Some(handoff) = result.handoff {
                        // Emit AgentDelegated event
                        self.deps
                            .event_sink
                            .emit(DomainEvent::new(DomainEventKind::AgentDelegated {
                                run_id: ctx.run_id.to_string(),
                                source_agent: handoff.from.0.to_string(),
                                target_agent: handoff.to.0.to_string(),
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
                                current_id = handoff.to;
                                continue;
                            }
                            HandoffMode::Delegate => {
                                // Execute target, then return to source
                                info!(
                                    from = %handoff.from,
                                    to = %handoff.to,
                                    "agent delegate"
                                );
                                let target_node = match graph.get_node(&handoff.to) {
                                    Some(n) => n,
                                    None => {
                                        warn!(to = %handoff.to, "delegate target not found");
                                        current_id = graph.entry.clone();
                                        continue;
                                    }
                                };
                                {
                                    use orka_core::stream::{StreamChunk, StreamChunkKind};
                                    self.deps.stream_registry.send(StreamChunk::new(
                                        ctx.session_id,
                                        ctx.trigger.channel.clone(),
                                        Some(ctx.trigger.id),
                                        StreamChunkKind::AgentSwitch {
                                            agent_id: target_node.agent.id.0.to_string(),
                                            display_name: target_node.agent.display_name.clone(),
                                        },
                                    ));
                                }
                                let delegate_result =
                                    run_agent_node(&target_node.agent, ctx, &self.deps, graph)
                                        .await?;
                                total_iterations += delegate_result.iterations;

                                if let Some(resp) = delegate_result.response {
                                    // Feed the delegate result back as a tool result message
                                    ctx.push_message(orka_llm::client::ChatMessageExt::user(
                                        format!("[Delegate result from {}]: {resp}", handoff.to),
                                    ))
                                    .await;
                                }

                                // Source agent continues
                                continue;
                            }
                        }
                    }

                    if let Some(resp) = result.response {
                        final_response = resp.clone();

                        // Check if this is a terminal agent
                        if policy.terminal_agents.contains(&current_id)
                            || graph.outgoing_edges(&current_id).is_empty()
                        {
                            break;
                        }

                        // Evaluate outgoing edges
                        let next = self.evaluate_edges(graph, &current_id, &resp, ctx).await;
                        match next {
                            Some(next_id) => {
                                current_id = next_id;
                                continue;
                            }
                            None => {
                                // No matching edge — terminal
                                break;
                            }
                        }
                    }
                }

                NodeKind::Router => {
                    // Router: evaluate edges without LLM, pick first matching
                    let edges = graph.outgoing_edges(&current_id);
                    let mut routed = false;
                    for edge in edges {
                        if self.edge_matches(edge, "", ctx).await {
                            current_id = edge.target.clone();
                            routed = true;
                            break;
                        }
                    }
                    if !routed {
                        break;
                    }
                }

                NodeKind::FanOut => {
                    // Fan-out: run all successors in parallel
                    let edges = graph.outgoing_edges(&current_id);
                    let mut join_set = tokio::task::JoinSet::new();

                    for edge in edges {
                        let target_id = edge.target.clone();
                        let target_node = match graph.get_node(&target_id) {
                            Some(n) => n.clone(),
                            None => continue,
                        };
                        let ctx = ctx.clone();
                        let deps = self.deps.clone();
                        let graph_clone = graph.clone();

                        join_set.spawn(async move {
                            {
                                use orka_core::stream::{StreamChunk, StreamChunkKind};
                                deps.stream_registry.send(StreamChunk::new(
                                    ctx.session_id,
                                    ctx.trigger.channel.clone(),
                                    Some(ctx.trigger.id),
                                    StreamChunkKind::AgentSwitch {
                                        agent_id: target_node.agent.id.0.to_string(),
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
                                if let Some(resp) = node_result.response {
                                    final_response = resp;
                                }
                            }
                            Ok(Err(e)) => warn!(%e, "fan-out agent failed"),
                            Err(e) => warn!(%e, "fan-out task panicked"),
                        }
                    }

                    // After fan-out, check for further edges from this node
                    break;
                }

                NodeKind::FanIn => {
                    // FanIn: same as Agent but reads merged results from context
                    {
                        use orka_core::stream::{StreamChunk, StreamChunkKind};
                        self.deps.stream_registry.send(StreamChunk::new(
                            ctx.session_id,
                            ctx.trigger.channel.clone(),
                            Some(ctx.trigger.id),
                            StreamChunkKind::AgentSwitch {
                                agent_id: node.agent.id.0.to_string(),
                                display_name: node.agent.display_name.clone(),
                            },
                        ));
                    }
                    let result = run_agent_node(&node.agent, ctx, &self.deps, graph)
                        .instrument(agent_span)
                        .await?;
                    total_iterations += result.iterations;
                    if let Some(resp) = result.response {
                        final_response = resp;
                    }
                    break;
                }
            }
        }

        // Emit stream Done chunk
        use orka_core::stream::{StreamChunk, StreamChunkKind};
        self.deps.stream_registry.send(StreamChunk::new(
            ctx.session_id,
            ctx.trigger.channel.clone(),
            Some(ctx.trigger.id),
            StreamChunkKind::Done,
        ));

        Ok(ExecutionResult {
            response: final_response,
            agents_executed,
            total_iterations,
            total_tokens: ctx.total_tokens(),
            duration_ms: start.elapsed().as_millis() as u64,
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
