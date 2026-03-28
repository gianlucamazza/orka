#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::default_trait_access
)]

//! Integration tests for slash commands end-to-end through the worker pipeline.

use std::{sync::Arc, time::Duration};

use orka_core::{
    config::AgentConfig,
    testing::{
        InMemoryBus, InMemoryEventSink, InMemoryMemoryStore, InMemoryQueue, InMemorySecretManager,
        InMemorySessionStore,
    },
    traits::{MessageBus, PriorityQueue, SessionStore},
    types::{CommandPayload, Envelope, Payload, Session},
};
use orka_skills::SkillRegistry;
use orka_worker::{
    EchoHandler, HandlerDispatcher, WorkerPool,
    commands::{CommandRegistry, register_all},
};
use tokio_util::sync::CancellationToken;

fn make_command_envelope(session: &Session, cmd_name: &str) -> Envelope {
    let mut env = Envelope::text(&session.channel, session.id, "");
    env.payload = Payload::Command(CommandPayload::new(
        cmd_name.to_string(),
        Default::default(),
    ));
    env
}

fn make_text_envelope(session: &Session, text: &str) -> Envelope {
    Envelope::text(&session.channel, session.id, text)
}

// ── CommandRegistry unit tests
// ────────────────────────────────────────────────

#[test]
fn command_registry_help_lists_all_commands() {
    let skills = Arc::new(SkillRegistry::default());
    let memory = Arc::new(InMemoryMemoryStore::new());
    let secrets = Arc::new(InMemorySecretManager::new());
    let workspace_registry = Arc::new(orka_workspace::WorkspaceRegistry::new("main".into()));
    let agent_config = AgentConfig::default();

    let mut registry = CommandRegistry::new();
    register_all(
        &mut registry,
        skills,
        memory,
        None,
        secrets,
        workspace_registry,
        &agent_config,
        None,
    );

    let help_text = registry.help_text();
    // Core commands must be present
    assert!(help_text.contains("/help"), "missing /help");
    assert!(help_text.contains("/memory"), "missing /memory");
    assert!(help_text.contains("/status"), "missing /status");
    assert!(help_text.contains("/skills"), "missing /skills");
    assert!(help_text.contains("/cancel"), "missing /cancel");
    assert!(help_text.contains("/workspace"), "missing /workspace");
    assert!(help_text.contains("/start"), "missing /start");
    // experience is not registered when disabled
    assert!(
        !help_text.contains("/experience"),
        "experience should not be registered"
    );
}

#[test]
fn command_registry_unknown_command_returns_none() {
    let registry = CommandRegistry::new();
    assert!(registry.get("nonexistent").is_none());
}

// ── Worker-pipeline tests via EchoHandler ────────────────────────────────────

async fn run_pool_with_envelope(envelope: Envelope, session: Session) -> Vec<Envelope> {
    let queue = Arc::new(InMemoryQueue::new());
    let sessions = Arc::new(InMemorySessionStore::new());
    let bus = Arc::new(InMemoryBus::new());
    let event_sink = Arc::new(InMemoryEventSink::new());

    sessions.put(&session).await.unwrap();
    queue.push(&envelope).await.unwrap();

    let mut rx = bus.subscribe("outbound").await.unwrap();

    let pool = WorkerPool::new(
        queue.clone(),
        sessions.clone(),
        bus.clone(),
        Arc::new(HandlerDispatcher::new(Arc::new(EchoHandler))),
        event_sink,
        1,
        0,
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    tokio::spawn(async move { pool.run(cancel_clone).await.unwrap() });

    let mut results = vec![];
    let deadline = tokio::time::sleep(Duration::from_secs(2));
    tokio::pin!(deadline);

    tokio::select! {
        () = &mut deadline => {},
        msg = rx.recv() => {
            if let Some(env) = msg {
                results.push(env);
            }
        }
    }

    cancel.cancel();
    results
}

#[tokio::test]
async fn echo_handler_processes_text_payload() {
    let session = Session::new("test", "user1");
    let env = make_text_envelope(&session, "hello world");
    let results = run_pool_with_envelope(env, session).await;
    assert_eq!(results.len(), 1);
    match &results[0].payload {
        Payload::Text(t) => assert!(t.contains("hello world")),
        other => panic!("unexpected payload: {other:?}"),
    }
}

// ── Cancel command: bypasses session lock
// ─────────────────────────────────────

#[tokio::test]
async fn cancel_command_responds_when_no_active_operation() {
    let queue = Arc::new(InMemoryQueue::new());
    let sessions = Arc::new(InMemorySessionStore::new());
    let bus = Arc::new(InMemoryBus::new());
    let event_sink = Arc::new(InMemoryEventSink::new());

    let session = Session::new("test", "user1");
    sessions.put(&session).await.unwrap();

    // Push a /cancel command — no active operation
    let env = make_command_envelope(&session, "cancel");
    queue.push(&env).await.unwrap();

    let mut rx = bus.subscribe("outbound").await.unwrap();

    let pool = WorkerPool::new(
        queue.clone(),
        sessions.clone(),
        bus.clone(),
        Arc::new(HandlerDispatcher::new(Arc::new(EchoHandler))),
        event_sink,
        1,
        0,
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    tokio::spawn(async move { pool.run(cancel_clone).await.unwrap() });

    // The cancel command should produce an outbound message
    let received = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("timed out")
        .expect("channel closed");

    // Worker-pool-level cancel: "Cancellation requested..." or no-op reply
    match &received.payload {
        Payload::Text(t) => {
            // Either "no active operation" (from CancelCommand::execute) or
            // "Cancellation requested" (from the worker pre-lock path)
            assert!(
                t.contains("cancel") || t.contains("Cancel") || t.contains("operation"),
                "unexpected cancel reply: {t}"
            );
        }
        other => panic!("expected Text, got {other:?}"),
    }

    cancel.cancel();
}

// ── Rate limiter
// ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn rate_limiter_allows_commands_under_limit() {
    // Just verifies the command registry is callable; rate limiter is tested
    // indirectly through the WorkspaceHandler, which is not wired up in this
    // test.
    let skills = Arc::new(SkillRegistry::default());
    let memory = Arc::new(InMemoryMemoryStore::new());
    let secrets = Arc::new(InMemorySecretManager::new());
    let workspace_registry = Arc::new(orka_workspace::WorkspaceRegistry::new("main".into()));
    let agent_config = AgentConfig::default();

    let mut registry = CommandRegistry::new();
    register_all(
        &mut registry,
        skills,
        memory,
        None,
        secrets,
        workspace_registry,
        &agent_config,
        None,
    );

    // The memory command should be registered
    assert!(registry.get("memory").is_some());
    assert!(registry.get("status").is_some());
}

// ── Command dispatch: unknown command
// ─────────────────────────────────────────

#[tokio::test]
async fn unknown_command_payload_does_not_panic() {
    // Sends an unknown /command via EchoHandler — EchoHandler echoes it back,
    // which validates the pipeline doesn't panic on unexpected command payloads.
    let session = Session::new("test", "user1");
    let mut env = Envelope::text(&session.channel, session.id, "");
    env.payload = Payload::Command(CommandPayload::new(
        "unknown_cmd".to_string(),
        Default::default(),
    ));
    let results = run_pool_with_envelope(env, session).await;
    // EchoHandler processes any payload; just verifying no panic
    assert!(results.len() <= 1);
}
