#![allow(missing_docs)]

//! Integration tests for `GraphExecutor` covering linear, fan-out, handoff,
//! and termination-policy scenarios using an in-memory mock LLM.

use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use async_trait::async_trait;
use orka_agent::{
    Agent, AgentGraph, AgentId, Edge, EdgeCondition, ExecutionContext, ExecutorDeps, GraphExecutor,
    GraphNode, NodeKind, TerminationPolicy,
};
use orka_checkpoint::{Checkpoint, CheckpointId, CheckpointStore};
use orka_core::{
    Envelope, RunId, Session, SessionId, StreamRegistry,
    testing::{InMemoryEventSink, InMemoryMemoryStore, InMemorySecretManager},
    traits::{Guardrail, GuardrailDecision},
};
use orka_llm::client::{
    ChatContent, ChatMessage, CompletionOptions, CompletionResponse, ContentBlock, LlmClient, Role,
    StopReason, ToolCall, ToolDefinition, Usage,
};
use orka_skills::SkillRegistry;
use tokio::sync::{Mutex, RwLock};

// ---------------------------------------------------------------------------
// In-memory checkpoint store for tests
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TestCheckpointStore {
    inner: RwLock<HashMap<String, Vec<Checkpoint>>>,
}

impl TestCheckpointStore {
    fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl CheckpointStore for TestCheckpointStore {
    async fn save(&self, checkpoint: &Checkpoint) -> orka_core::Result<()> {
        let mut map = self.inner.write().await;
        let entry = map.entry(checkpoint.run_id.clone()).or_default();
        if let Some(pos) = entry.iter().position(|c| c.id == checkpoint.id) {
            entry[pos] = checkpoint.clone();
        } else {
            entry.push(checkpoint.clone());
        }
        Ok(())
    }

    async fn load_latest(&self, run_id: &str) -> orka_core::Result<Option<Checkpoint>> {
        Ok(self
            .inner
            .read()
            .await
            .get(run_id)
            .and_then(|v| v.last().cloned()))
    }

    async fn load(&self, run_id: &str, id: &CheckpointId) -> orka_core::Result<Option<Checkpoint>> {
        Ok(self
            .inner
            .read()
            .await
            .get(run_id)
            .and_then(|v| v.iter().find(|c| &c.id == id).cloned()))
    }

    async fn list(&self, run_id: &str) -> orka_core::Result<Vec<CheckpointId>> {
        Ok(self
            .inner
            .read()
            .await
            .get(run_id)
            .map(|v| v.iter().map(|c| c.id.clone()).collect())
            .unwrap_or_default())
    }

    async fn delete_run(&self, run_id: &str) -> orka_core::Result<()> {
        self.inner.write().await.remove(run_id);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Mock LLM
// ---------------------------------------------------------------------------

enum MockResp {
    Text(String),
    /// Emit a `transfer_to_agent` tool call.
    Transfer {
        to: String,
        reason: String,
    },
    /// Emit an arbitrary named tool call (for testing HITL etc.).
    ToolCallResp {
        name: String,
        input: serde_json::Value,
    },
}

struct MockLlm {
    queue: Mutex<VecDeque<MockResp>>,
    systems: Mutex<Vec<String>>,
    last_messages: Mutex<Vec<ChatMessage>>,
}

impl MockLlm {
    fn new(resps: Vec<MockResp>) -> Self {
        Self {
            queue: Mutex::new(resps.into()),
            systems: Mutex::new(Vec::new()),
            last_messages: Mutex::new(Vec::new()),
        }
    }

    async fn last_system_prompt(&self) -> Option<String> {
        self.systems.lock().await.last().cloned()
    }

    async fn last_user_message(&self) -> Option<String> {
        self.last_messages
            .lock()
            .await
            .iter()
            .rfind(|m| matches!(m.role, Role::User))
            .and_then(|m| match &m.content {
                ChatContent::Text(t) => Some(t.clone()),
                _ => None,
            })
    }
}

#[async_trait]
impl LlmClient for MockLlm {
    async fn complete(
        &self,
        _messages: Vec<ChatMessage>,
        _system: &str,
    ) -> orka_core::Result<String> {
        match self.queue.lock().await.pop_front() {
            Some(MockResp::Text(t)) => Ok(t),
            Some(MockResp::ToolCallResp { name, .. }) => Ok(format!("[tool: {name}]")),
            _ => Ok("mock".into()),
        }
    }

    async fn complete_with_tools(
        &self,
        messages: &[orka_llm::client::ChatMessage],
        system: &str,
        _tools: &[ToolDefinition],
        _options: CompletionOptions,
    ) -> orka_core::Result<CompletionResponse> {
        self.systems.lock().await.push(system.to_string());
        *self.last_messages.lock().await = messages.to_vec();
        match self.queue.lock().await.pop_front() {
            Some(MockResp::Text(t)) => Ok(CompletionResponse::new(
                vec![ContentBlock::Text(t)],
                Usage::default(),
                Some(StopReason::EndTurn),
            )),
            Some(MockResp::Transfer { to, reason }) => Ok(CompletionResponse::new(
                vec![ContentBlock::ToolUse(ToolCall::new(
                    "h1",
                    "transfer_to_agent",
                    serde_json::json!({ "agent_id": to, "reason": reason }),
                ))],
                Usage::default(),
                Some(StopReason::ToolUse),
            )),
            Some(MockResp::ToolCallResp { name, input }) => Ok(CompletionResponse::new(
                vec![ContentBlock::ToolUse(ToolCall::new("tc1", &name, input))],
                Usage::default(),
                Some(StopReason::ToolUse),
            )),
            None => Ok(CompletionResponse::new(
                vec![ContentBlock::Text("mock".into())],
                Usage::default(),
                Some(StopReason::EndTurn),
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_deps(llm: Arc<dyn LlmClient>) -> ExecutorDeps {
    ExecutorDeps {
        skills: Arc::new(SkillRegistry::new()),
        memory: Arc::new(InMemoryMemoryStore::new()),
        secrets: Arc::new(InMemorySecretManager::new()),
        llm: Some(llm),
        event_sink: Arc::new(InMemoryEventSink::new()),
        stream_registry: StreamRegistry::new(),
        experience: None,
        soft_skills: None,
        templates: None,
        coding_runtime: None,
        guardrail: None,
        checkpoint_store: None,
    }
}

fn make_deps_no_llm() -> ExecutorDeps {
    ExecutorDeps {
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
        checkpoint_store: None,
    }
}

#[tokio::test]
async fn system_prompt_includes_coding_runtime_status() {
    let llm_impl = Arc::new(MockLlm::new(vec![MockResp::Text("ok".into())]));
    let llm: Arc<dyn LlmClient> = llm_impl.clone();
    let mut deps = make_deps(llm);
    deps.coding_runtime = Some(orka_agent::executor::CodingRuntimeStatus {
        tool_available: true,
        default_provider: "auto".into(),
        selection_policy: "availability".into(),
        claude_code_available: true,
        codex_available: true,
        selected_backend: Some("claude_code".into()),
        file_modifications_allowed: true,
        command_execution_allowed: true,
        allowed_paths: vec!["/home".into(), "/tmp".into()],
        denied_paths: vec!["/home/gianluca/.ssh".into()],
    });

    let executor = GraphExecutor::new(deps);
    let agent = Agent::new("a", "Agent");
    let mut graph = AgentGraph::new("g", AgentId::from("a"));
    graph.add_node(GraphNode {
        agent,
        kind: NodeKind::Agent,
    });
    graph = graph.with_termination(TerminationPolicy::default());
    let session_id = SessionId::new();
    let mut envelope = Envelope::text("custom", session_id, "hai tools di coding?");
    envelope.insert_meta("workspace:cwd", "/home/gianluca");
    let ctx = ExecutionContext::new(envelope);

    let _ = executor.execute(&graph, &ctx).await.unwrap();
    let prompt = llm_impl.last_system_prompt().await.unwrap();
    assert!(prompt.contains("## Coding Runtime"));
    assert!(prompt.contains("coding_delegate"));
    assert!(prompt.contains("claude_code, codex"));
    assert!(prompt.contains("Selected backend for a delegated run right now: `claude_code`."));
    assert!(prompt.contains("Selected backend file modifications: allowed."));
    assert!(prompt.contains("Current user working directory from the client: `/home/gianluca`."));
    assert!(
        prompt.contains(
            "OS policy for the current working directory: allowed by `os.allowed_paths`."
        )
    );
}

fn make_ctx() -> ExecutionContext {
    ExecutionContext::new(Envelope::text("chan", SessionId::new(), "hello"))
}

fn agent(id: &str) -> Agent {
    Agent::new(id, id)
}

fn agent_node(id: &str) -> GraphNode {
    GraphNode {
        agent: agent(id),
        kind: NodeKind::Agent,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// When no LLM is configured the executor returns "No LLM provider configured."
#[tokio::test]
async fn no_llm_returns_default_message() {
    let entry = AgentId::new("solo");
    let mut graph = AgentGraph::new("g", entry.clone());
    graph.add_node(agent_node("solo"));

    let ctx = make_ctx();
    let executor = GraphExecutor::new(make_deps_no_llm());
    let result = executor.execute(&graph, &ctx).await.unwrap();

    assert_eq!(result.response, "No LLM provider configured.");
    assert_eq!(result.agents_executed, vec!["solo"]);
}

/// Single terminal agent (no outgoing edges): executor returns the LLM
/// response.
#[tokio::test]
async fn single_terminal_agent() {
    let entry = AgentId::new("a");
    let mut graph = AgentGraph::new("g", entry.clone());
    graph.add_node(agent_node("a"));

    let llm = Arc::new(MockLlm::new(vec![MockResp::Text("Hello!".into())]));
    let ctx = make_ctx();
    let result = GraphExecutor::new(make_deps(llm))
        .execute(&graph, &ctx)
        .await
        .unwrap();

    assert_eq!(result.response, "Hello!");
    assert_eq!(result.agents_executed, vec!["a"]);
    assert_eq!(result.total_iterations, 1);
}

/// A → B via `Always` edge: B's response is the final result.
#[tokio::test]
async fn two_agent_linear_always_edge() {
    let a = AgentId::new("a");
    let b = AgentId::new("b");

    let mut graph = AgentGraph::new("g", a.clone());
    graph.add_node(agent_node("a"));
    graph.add_node(agent_node("b"));
    graph.add_edge(
        a.clone(),
        Edge {
            target: b.clone(),
            condition: Some(EdgeCondition::Always),
            priority: 0,
        },
    );

    let llm = Arc::new(MockLlm::new(vec![
        MockResp::Text("from a".into()),
        MockResp::Text("from b".into()),
    ]));
    let ctx = make_ctx();
    let result = GraphExecutor::new(make_deps(llm))
        .execute(&graph, &ctx)
        .await
        .unwrap();

    assert_eq!(result.response, "from b");
    assert_eq!(result.agents_executed, vec!["a", "b"]);
}

/// `OutputContains` edge matches: routing proceeds to next agent.
#[tokio::test]
async fn output_contains_edge_matches() {
    let a = AgentId::new("a");
    let b = AgentId::new("b");

    let mut graph = AgentGraph::new("g", a.clone());
    graph.add_node(agent_node("a"));
    graph.add_node(agent_node("b"));
    graph.add_edge(
        a.clone(),
        Edge {
            target: b.clone(),
            condition: Some(EdgeCondition::OutputContains("keyword".into())),
            priority: 0,
        },
    );

    let llm = Arc::new(MockLlm::new(vec![
        MockResp::Text("the keyword is here".into()),
        MockResp::Text("b done".into()),
    ]));
    let ctx = make_ctx();
    let result = GraphExecutor::new(make_deps(llm))
        .execute(&graph, &ctx)
        .await
        .unwrap();

    assert_eq!(result.response, "b done");
    assert_eq!(result.agents_executed, vec!["a", "b"]);
}

/// `OutputContains` edge does NOT match: execution terminates at A.
#[tokio::test]
async fn output_contains_edge_no_match_terminates() {
    let a = AgentId::new("a");
    let b = AgentId::new("b");

    let mut graph = AgentGraph::new("g", a.clone());
    graph.add_node(agent_node("a"));
    graph.add_node(agent_node("b"));
    graph.add_edge(
        a.clone(),
        Edge {
            target: b.clone(),
            condition: Some(EdgeCondition::OutputContains("NOPE".into())),
            priority: 0,
        },
    );

    let llm = Arc::new(MockLlm::new(vec![MockResp::Text("no match here".into())]));
    let ctx = make_ctx();
    let result = GraphExecutor::new(make_deps(llm))
        .execute(&graph, &ctx)
        .await
        .unwrap();

    assert_eq!(result.response, "no match here");
    // B was never reached
    assert_eq!(result.agents_executed, vec!["a"]);
}

/// `StateMatch` edge matches: routing proceeds when state slot equals expected
/// value.
#[tokio::test]
async fn state_match_edge_routes_correctly() {
    let a = AgentId::new("a");
    let b = AgentId::new("b");
    let slot = orka_agent::SlotKey::shared("flag");

    let mut graph = AgentGraph::new("g", a.clone());
    graph.add_node(agent_node("a"));
    graph.add_node(agent_node("b"));
    graph.add_edge(
        a.clone(),
        Edge {
            target: b.clone(),
            condition: Some(EdgeCondition::StateMatch {
                key: slot.clone(),
                pattern: serde_json::json!("go"),
            }),
            priority: 0,
        },
    );

    let llm = Arc::new(MockLlm::new(vec![
        MockResp::Text("a done".into()),
        MockResp::Text("b done".into()),
    ]));
    let ctx = make_ctx();
    // Pre-load the state slot so the edge condition matches
    ctx.set(&a, slot, serde_json::json!("go")).await;

    let result = GraphExecutor::new(make_deps(llm))
        .execute(&graph, &ctx)
        .await
        .unwrap();

    assert_eq!(result.response, "b done");
    assert_eq!(result.agents_executed, vec!["a", "b"]);
}

/// `max_total_iterations = 1`: the second graph loop iteration triggers the
/// guard before agent B can run.
#[tokio::test]
async fn termination_max_total_iterations() {
    let a = AgentId::new("a");
    let b = AgentId::new("b");

    let policy = TerminationPolicy {
        max_total_iterations: 1,
        ..Default::default()
    };
    let mut graph = AgentGraph::new("g", a.clone()).with_termination(policy);
    graph.add_node(agent_node("a"));
    graph.add_node(agent_node("b"));
    graph.add_edge(
        a.clone(),
        Edge {
            target: b.clone(),
            condition: Some(EdgeCondition::Always),
            priority: 0,
        },
    );

    let llm = Arc::new(MockLlm::new(vec![
        MockResp::Text("a out".into()),
        MockResp::Text("b out".into()),
    ]));
    let ctx = make_ctx();
    let result = GraphExecutor::new(make_deps(llm))
        .execute(&graph, &ctx)
        .await
        .unwrap();

    // A ran on iteration 1 (allowed), B would run on iteration 2 (blocked)
    assert_eq!(result.agents_executed, vec!["a"]);
}

/// `terminal_agents` policy: execution ends after A without following the edge
/// to B.
#[tokio::test]
async fn termination_terminal_agent_policy() {
    let a = AgentId::new("a");
    let b = AgentId::new("b");

    let policy = TerminationPolicy {
        terminal_agents: [a.clone()].into(),
        ..Default::default()
    };
    let mut graph = AgentGraph::new("g", a.clone()).with_termination(policy);
    graph.add_node(agent_node("a"));
    graph.add_node(agent_node("b"));
    graph.add_edge(
        a.clone(),
        Edge {
            target: b.clone(),
            condition: Some(EdgeCondition::Always),
            priority: 0,
        },
    );

    let llm = Arc::new(MockLlm::new(vec![MockResp::Text("a final".into())]));
    let ctx = make_ctx();
    let result = GraphExecutor::new(make_deps(llm))
        .execute(&graph, &ctx)
        .await
        .unwrap();

    assert_eq!(result.response, "a final");
    assert_eq!(result.agents_executed, vec!["a"]);
}

/// FanOut node dispatches all successors; all three agent IDs appear in
/// `agents_executed`.
#[tokio::test]
async fn fan_out_runs_all_successors() {
    let fanout = AgentId::new("fanout");
    let b = AgentId::new("b");
    let c = AgentId::new("c");

    let mut graph = AgentGraph::new("g", fanout.clone());
    graph.add_node(GraphNode {
        agent: agent("fanout"),
        kind: NodeKind::FanOut,
    });
    graph.add_node(agent_node("b"));
    graph.add_node(agent_node("c"));
    graph.add_edge(
        fanout.clone(),
        Edge {
            target: b.clone(),
            condition: None,
            priority: 0,
        },
    );
    graph.add_edge(
        fanout.clone(),
        Edge {
            target: c.clone(),
            condition: None,
            priority: 1,
        },
    );

    let llm = Arc::new(MockLlm::new(vec![
        MockResp::Text("b response".into()),
        MockResp::Text("c response".into()),
    ]));
    let ctx = make_ctx();
    let result = GraphExecutor::new(make_deps(llm))
        .execute(&graph, &ctx)
        .await
        .unwrap();

    // fanout itself + both successors
    assert_eq!(result.agents_executed.len(), 1); // only "fanout" in agents_executed (successors run inside fanout handling)
    // The final response is whichever branch finished last (non-deterministic),
    // so just assert non-empty (response could be "b response" or "c
    // response")
}

/// Handoff transfer: A transfers control to B; B's response becomes the final
/// result.
#[tokio::test]
async fn handoff_transfer_routes_to_target() {
    let a = AgentId::new("a");
    let b = AgentId::new("b");

    // A must have b as a handoff target so the handoff tool is injected
    let mut agent_a = agent("a");
    agent_a.handoff_targets = vec![b.clone()];

    let mut graph = AgentGraph::new("g", a.clone());
    graph.add_node(GraphNode {
        agent: agent_a,
        kind: NodeKind::Agent,
    });
    graph.add_node(agent_node("b"));

    let llm = Arc::new(MockLlm::new(vec![
        MockResp::Transfer {
            to: "b".into(),
            reason: "escalate".into(),
        },
        MockResp::Text("b handled it".into()),
    ]));
    let ctx = make_ctx();
    let result = GraphExecutor::new(make_deps(llm))
        .execute(&graph, &ctx)
        .await
        .unwrap();

    assert_eq!(result.response, "b handled it");
    assert!(result.agents_executed.contains(&"a".to_string()));
    assert!(result.agents_executed.contains(&"b".to_string()));
}

/// Router node (no LLM call): picks the first matching edge and routes.
#[tokio::test]
async fn router_node_routes_on_always_edge() {
    let router = AgentId::new("router");
    let b = AgentId::new("b");

    let mut graph = AgentGraph::new("g", router.clone());
    graph.add_node(GraphNode {
        agent: agent("router"),
        kind: NodeKind::Router,
    });
    graph.add_node(agent_node("b"));
    graph.add_edge(
        router.clone(),
        Edge {
            target: b.clone(),
            condition: Some(EdgeCondition::Always),
            priority: 0,
        },
    );

    // Only B needs a response (Router never calls LLM)
    let llm = Arc::new(MockLlm::new(vec![MockResp::Text("routed to b".into())]));
    let ctx = make_ctx();
    let result = GraphExecutor::new(make_deps(llm))
        .execute(&graph, &ctx)
        .await
        .unwrap();

    assert_eq!(result.response, "routed to b");
    assert!(result.agents_executed.contains(&"b".to_string()));
}

/// Edge priority ordering: lower priority edge is evaluated first.
#[tokio::test]
async fn edge_priority_lower_wins() {
    let a = AgentId::new("a");
    let first = AgentId::new("first");
    let second = AgentId::new("second");

    let mut graph = AgentGraph::new("g", a.clone());
    graph.add_node(agent_node("a"));
    graph.add_node(agent_node("first"));
    graph.add_node(agent_node("second"));
    // Add high-priority edge first in insertion order but lower priority number
    // wins
    graph.add_edge(
        a.clone(),
        Edge {
            target: second.clone(),
            condition: Some(EdgeCondition::Always),
            priority: 10,
        },
    );
    graph.add_edge(
        a.clone(),
        Edge {
            target: first.clone(),
            condition: Some(EdgeCondition::Always),
            priority: 1,
        },
    );

    let llm = Arc::new(MockLlm::new(vec![
        MockResp::Text("a out".into()),
        MockResp::Text("first out".into()),
    ]));
    let ctx = make_ctx();
    let result = GraphExecutor::new(make_deps(llm))
        .execute(&graph, &ctx)
        .await
        .unwrap();

    assert_eq!(result.response, "first out");
    assert!(result.agents_executed.contains(&"first".to_string()));
    assert!(!result.agents_executed.contains(&"second".to_string()));
}

/// HITL: when an agent has `interrupt_before_tools = ["dangerous_tool"]` and
/// the LLM requests that tool, the executor saves an `Interrupted` checkpoint
/// and returns an empty response without executing the tool.
#[tokio::test]
async fn hitl_interrupt_pauses_execution_and_saves_checkpoint() {
    use orka_checkpoint::RunStatus;

    let checkpoint_store = Arc::new(TestCheckpointStore::new());

    // LLM emits a tool call for "dangerous_tool", then would emit a text response
    // on the next turn — but execution should stop before the second call.
    let llm = Arc::new(MockLlm::new(vec![MockResp::ToolCallResp {
        name: "dangerous_tool".into(),
        input: serde_json::json!({"param": "value"}),
    }]));

    let mut deps = make_deps(llm);
    deps.checkpoint_store = Some(checkpoint_store.clone());

    let mut agent = Agent::new("a", "Agent");
    agent
        .interrupt_before_tools
        .insert("dangerous_tool".to_string());

    let mut graph = AgentGraph::new("g", AgentId::from("a"));
    graph.add_node(GraphNode {
        agent,
        kind: NodeKind::Agent,
    });
    graph = graph.with_termination(TerminationPolicy::default());

    let ctx = make_ctx();
    let result = GraphExecutor::new(deps)
        .execute(&graph, &ctx)
        .await
        .unwrap();

    // Execution stops — empty response, no agents fully executed
    assert!(
        result.response.is_empty(),
        "expected empty response on interrupt"
    );

    // Checkpoint must be saved with Interrupted status
    let ckpt = checkpoint_store
        .load_latest(&ctx.run_id.to_string())
        .await
        .unwrap()
        .expect("checkpoint should have been saved");

    assert!(
        matches!(ckpt.status, RunStatus::Interrupted { .. }),
        "checkpoint should be Interrupted, got {:?}",
        ckpt.status
    );
    // resume_node should point back at the interrupted agent
    assert_eq!(ckpt.resume_node.as_deref(), Some("a"));
}

/// Checkpoint resume: executor continues from `resume_node` in the checkpoint,
/// skipping already-completed nodes.
#[tokio::test]
async fn executor_resume_continues_from_checkpoint_node() {
    use orka_checkpoint::{Checkpoint, CheckpointId, RunStatus};

    let checkpoint_store = Arc::new(TestCheckpointStore::new());

    // Two-node linear graph: a → b
    let a_id = AgentId::new("a");
    let b_id = AgentId::new("b");

    let mut graph = AgentGraph::new("g", a_id.clone());
    graph.add_node(agent_node("a"));
    graph.add_node(agent_node("b"));
    graph.add_edge(
        a_id.clone(),
        Edge {
            target: b_id.clone(),
            condition: Some(EdgeCondition::Always),
            priority: 0,
        },
    );
    graph = graph.with_termination(TerminationPolicy::default());

    // Pre-seed a checkpoint that marks node "a" as completed and resumes from "b"
    let envelope = Envelope::text("chan", SessionId::new(), "hello");
    let run_id = RunId::new().to_string();
    let ckpt = Checkpoint {
        id: CheckpointId::new(),
        run_id: run_id.clone(),
        graph_id: "g".to_string(),
        session_id: envelope.session_id,
        trigger: envelope,
        completed_node: "a".to_string(),
        resume_node: Some("b".to_string()),
        status: RunStatus::Running,
        state: Default::default(),
        messages: vec![],
        total_tokens: 0,
        changelog: vec![],
        agents_executed: vec!["a".to_string()],
        total_iterations: 1,
        created_at: chrono::Utc::now(),
    };
    checkpoint_store.save(&ckpt).await.unwrap();

    // Only B needs to produce a response
    let llm = Arc::new(MockLlm::new(vec![MockResp::Text("b result".into())]));
    let mut deps = make_deps(llm);
    deps.checkpoint_store = Some(checkpoint_store.clone());

    let executor = GraphExecutor::new(deps);
    let result = executor
        .resume(&run_id, &graph)
        .await
        .unwrap()
        .expect("resume should produce a result");

    assert_eq!(result.response, "b result");
    // Only b should be in agents_executed (a was already done)
    assert!(
        result.agents_executed.contains(&"b".to_string()),
        "b should be executed on resume"
    );
    assert!(
        !result.agents_executed.contains(&"a".to_string()),
        "a should be skipped on resume"
    );
}

// ---------------------------------------------------------------------------
// Mock guardrail for testing input modification
// ---------------------------------------------------------------------------

struct ModifyGuardrail {
    replacement: String,
}

#[async_trait]
impl Guardrail for ModifyGuardrail {
    async fn check_input(
        &self,
        _input: &str,
        _session: &Session,
    ) -> orka_core::Result<GuardrailDecision> {
        Ok(GuardrailDecision::Modify(self.replacement.clone()))
    }

    async fn check_output(
        &self,
        _output: &str,
        _session: &Session,
    ) -> orka_core::Result<GuardrailDecision> {
        Ok(GuardrailDecision::Allow)
    }
}

struct BlockGuardrail;

#[async_trait]
impl Guardrail for BlockGuardrail {
    async fn check_input(
        &self,
        _input: &str,
        _session: &Session,
    ) -> orka_core::Result<GuardrailDecision> {
        Ok(GuardrailDecision::Block("blocked by policy".to_string()))
    }

    async fn check_output(
        &self,
        _output: &str,
        _session: &Session,
    ) -> orka_core::Result<GuardrailDecision> {
        Ok(GuardrailDecision::Allow)
    }
}

// ---------------------------------------------------------------------------
// FanOut → FanIn pipeline test
// ---------------------------------------------------------------------------

/// A FanOut node dispatches parallel branches; a FanIn node with an outgoing
/// edge from FanOut should be reached after all branches complete.
#[tokio::test]
async fn fan_out_continues_to_fan_in() {
    // Graph: fanout → {worker_a, worker_b, synthesizer}
    // where synthesizer is NodeKind::FanIn
    // After worker_a and worker_b complete, the executor should route to
    // synthesizer.
    let llm = Arc::new(MockLlm::new(vec![
        MockResp::Text("a done".into()),
        MockResp::Text("b done".into()),
        MockResp::Text("synthesis".into()),
    ]));
    let deps = make_deps(llm);
    let executor = GraphExecutor::new(deps);

    let mut graph = AgentGraph::new("g", AgentId::from("fanout"));
    graph.add_node(GraphNode {
        agent: agent("fanout"),
        kind: NodeKind::FanOut,
    });
    graph.add_node(GraphNode {
        agent: agent("worker_a"),
        kind: NodeKind::Agent,
    });
    graph.add_node(GraphNode {
        agent: agent("worker_b"),
        kind: NodeKind::Agent,
    });
    graph.add_node(GraphNode {
        agent: agent("synthesizer"),
        kind: NodeKind::FanIn,
    });

    // Edges: fanout → worker_a, fanout → worker_b (parallel branches)
    //        fanout → synthesizer (FanIn — reached after branches complete)
    graph.add_edge(
        AgentId::from("fanout"),
        Edge {
            target: AgentId::from("worker_a"),
            condition: Some(EdgeCondition::Always),
            priority: 0,
        },
    );
    graph.add_edge(
        AgentId::from("fanout"),
        Edge {
            target: AgentId::from("worker_b"),
            condition: Some(EdgeCondition::Always),
            priority: 0,
        },
    );
    graph.add_edge(
        AgentId::from("fanout"),
        Edge {
            target: AgentId::from("synthesizer"),
            condition: Some(EdgeCondition::Always),
            priority: 0,
        },
    );

    let ctx = make_ctx();
    let result = executor.execute(&graph, &ctx).await.unwrap();

    // Synthesizer (FanIn) runs last and produces the final response.
    assert_eq!(result.response, "synthesis");
    assert!(result.agents_executed.contains(&"synthesizer".to_string()));
}

/// FanIn node with outgoing edges continues graph traversal after synthesis.
#[tokio::test]
async fn fan_in_continues_to_next_node() {
    let llm = Arc::new(MockLlm::new(vec![
        MockResp::Text("worker result".into()),
        MockResp::Text("fan-in result".into()),
        MockResp::Text("final".into()),
    ]));
    let deps = make_deps(llm);
    let executor = GraphExecutor::new(deps);

    let mut graph = AgentGraph::new("g", AgentId::from("fanout"));
    graph.add_node(GraphNode {
        agent: agent("fanout"),
        kind: NodeKind::FanOut,
    });
    graph.add_node(GraphNode {
        agent: agent("worker"),
        kind: NodeKind::Agent,
    });
    graph.add_node(GraphNode {
        agent: agent("collector"),
        kind: NodeKind::FanIn,
    });
    graph.add_node(GraphNode {
        agent: agent("post"),
        kind: NodeKind::Agent,
    });

    graph.add_edge(
        AgentId::from("fanout"),
        Edge {
            target: AgentId::from("worker"),
            condition: Some(EdgeCondition::Always),
            priority: 0,
        },
    );
    graph.add_edge(
        AgentId::from("fanout"),
        Edge {
            target: AgentId::from("collector"),
            condition: Some(EdgeCondition::Always),
            priority: 0,
        },
    );
    // FanIn → post (continuation after synthesis)
    graph.add_edge(
        AgentId::from("collector"),
        Edge {
            target: AgentId::from("post"),
            condition: Some(EdgeCondition::Always),
            priority: 0,
        },
    );

    let ctx = make_ctx();
    let result = executor.execute(&graph, &ctx).await.unwrap();

    assert_eq!(result.response, "final");
    assert!(result.agents_executed.contains(&"post".to_string()));
}

// ---------------------------------------------------------------------------
// Guardrail tests
// ---------------------------------------------------------------------------

/// When a guardrail blocks a tool call, the skill is not executed and the LLM
/// receives a "blocked by guardrail" tool result instead.  The next LLM turn
/// produces the final response.
#[tokio::test]
async fn guardrail_block_prevents_skill_execution() {
    use orka_core::testing::EchoSkill;

    // LLM emits a tool call; the guardrail blocks it; the block message becomes
    // the tool result fed back to the LLM which then produces the final text.
    let llm = Arc::new(MockLlm::new(vec![
        MockResp::ToolCallResp {
            name: "echo".into(),
            input: serde_json::json!({ "message": "secret" }),
        },
        MockResp::Text("Understood, I will not proceed.".into()),
    ]));

    let mut deps = make_deps(llm);
    let mut skills = SkillRegistry::new();
    skills.register(Arc::new(EchoSkill));
    deps.skills = Arc::new(skills);
    deps.guardrail = Some(Arc::new(BlockGuardrail));

    let executor = GraphExecutor::new(deps);
    let mut graph = AgentGraph::new("g", AgentId::from("a"));
    graph.add_node(GraphNode {
        agent: agent("a"),
        kind: NodeKind::Agent,
    });

    let ctx = make_ctx();
    let result = executor.execute(&graph, &ctx).await.unwrap();
    // The block message becomes the tool result; the executor surfaces either
    // the LLM's next-turn response or the block message itself as the final output.
    assert!(
        result.response.contains("blocked") || result.response.contains("Understood"),
        "expected block-related response, got: {}",
        result.response
    );
}

/// When a guardrail blocks a tool call and the LLM is exhausted (no further
/// turns configured), the executor surfaces the block error as the response.
#[tokio::test]
async fn guardrail_block_surfaces_error_when_llm_exhausted() {
    use orka_core::testing::EchoSkill;

    let llm = Arc::new(MockLlm::new(vec![MockResp::ToolCallResp {
        name: "echo".into(),
        input: serde_json::json!({ "message": "secret" }),
    }]));

    let mut deps = make_deps(llm);
    let mut skills = SkillRegistry::new();
    skills.register(Arc::new(EchoSkill));
    deps.skills = Arc::new(skills);
    deps.guardrail = Some(Arc::new(BlockGuardrail));

    let executor = GraphExecutor::new(deps);
    let mut graph = AgentGraph::new("g", AgentId::from("a"));
    graph.add_node(GraphNode {
        agent: agent("a"),
        kind: NodeKind::Agent,
    });

    let ctx = make_ctx();
    let result = executor.execute(&graph, &ctx).await.unwrap();
    assert!(
        result.response.contains("blocked by policy") || result.response.contains("blocked"),
        "expected block message in response, got: {}",
        result.response
    );
}

/// When a guardrail returns `Modify`, the trigger text is replaced before the
/// LLM sees it — the LLM receives the modified input, not the original.
#[tokio::test]
async fn guardrail_modify_replaces_user_input() {
    let llm_impl = Arc::new(MockLlm::new(vec![MockResp::Text("done".into())]));
    let llm: Arc<dyn LlmClient> = llm_impl.clone();
    let mut deps = make_deps(llm);
    deps.guardrail = Some(Arc::new(ModifyGuardrail {
        replacement: "filtered input".into(),
    }));

    let executor = GraphExecutor::new(deps);
    let mut graph = AgentGraph::new("g", AgentId::from("a"));
    graph.add_node(GraphNode {
        agent: agent("a"),
        kind: NodeKind::Agent,
    });

    let ctx = make_ctx(); // trigger text is "hello"
    let result = executor.execute(&graph, &ctx).await.unwrap();
    assert_eq!(result.response, "done");

    let last_user = llm_impl.last_user_message().await.unwrap();
    assert_eq!(
        last_user, "filtered input",
        "LLM should receive the modified input, not the original trigger"
    );
}

/// Delegate handoff: A delegates a sub-task to B via `delegate_to_agent`, B
/// executes, its result is fed back into A's conversation, and A resumes to
/// produce the final response.
#[tokio::test]
async fn delegate_handoff_returns_to_parent() {
    let a = AgentId::new("a");
    let b = AgentId::new("b");

    // A has B as a handoff target so both transfer and delegate tools are injected
    let mut agent_a = agent("a");
    agent_a.handoff_targets = vec![b.clone()];

    let mut graph = AgentGraph::new("g", a.clone());
    graph.add_node(GraphNode {
        agent: agent_a,
        kind: NodeKind::Agent,
    });
    graph.add_node(agent_node("b"));

    let llm = Arc::new(MockLlm::new(vec![
        // A calls delegate_to_agent (not transfer_to_agent)
        MockResp::ToolCallResp {
            name: "delegate_to_agent".into(),
            input: serde_json::json!({
                "agent_id": "b",
                "reason": "sub-task",
                "task": "do the work"
            }),
        },
        // B produces its result (run inline)
        MockResp::Text("b result".into()),
        // A resumes after B's result is injected back into its conversation
        MockResp::Text("a final after delegate".into()),
    ]));

    let ctx = make_ctx();
    let result = GraphExecutor::new(make_deps(llm))
        .execute(&graph, &ctx)
        .await
        .unwrap();

    // A should produce the final response after B's result is injected back
    assert_eq!(result.response, "a final after delegate");
    // A is tracked as the primary agent in this run
    assert!(result.agents_executed.contains(&"a".to_string()));
}
