//! Worker pool that consumes messages from the priority queue and dispatches to handlers.
//!
//! - [`WorkerPool`] — concurrent worker loop with retry and dead-letter queue support
//! - [`AgentHandler`] — trait for message handling implementations
//! - [`WorkspaceHandler`] — LLM-powered handler with skill execution and tool loops

#![warn(missing_docs)]

/// Slash command framework and built-in command implementations.
pub mod commands;
/// `AgentHandler` trait and the built-in `EchoHandler`.
pub mod handler;
/// Shared conversation-history helpers (compaction + persistence).
pub mod history;
/// Re-exports streaming types from `orka-core`.
pub mod stream;
/// LLM-powered agent handler with tool loops and guardrails.
pub mod workspace_handler;

// re-exports
pub use commands::CommandRegistry;
pub use handler::{AgentHandler, EchoHandler};
pub use stream::{StreamChunk, StreamChunkKind, StreamRegistry};
pub use workspace_handler::{WorkspaceHandler, WorkspaceHandlerConfig};

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use orka_agent::{AgentGraph, ExecutionContext, GraphExecutor};
use orka_core::traits::{EventSink, MemoryStore, MessageBus, PriorityQueue, SessionStore};
use orka_core::types::SessionId;
use orka_core::{DomainEvent, DomainEventKind, Envelope, Payload, Priority, Session};
use orka_llm::client::ChatMessage;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Shared map of per-session cancellation tokens.  The worker populates this before
/// each handler invocation; the `/cancel` command uses it to abort ongoing LLM loops.
pub type SessionCancelTokens = Arc<Mutex<HashMap<SessionId, CancellationToken>>>;

/// Concurrent worker pool that pops envelopes from a priority queue and dispatches them.
pub struct WorkerPool {
    queue: Arc<dyn PriorityQueue>,
    sessions: Arc<dyn SessionStore>,
    bus: Arc<dyn MessageBus>,
    handler: Arc<dyn AgentHandler>,
    event_sink: Arc<dyn EventSink>,
    session_lock: Option<Arc<dyn MemoryStore>>,
    concurrency: usize,
    max_retries: u32,
    retry_base_delay_ms: u64,
    /// Cancellation tokens shared between the worker loop and the cancel command.
    session_cancel_tokens: SessionCancelTokens,
}

impl WorkerPool {
    /// Create a new pool with the given concurrency level and retry policy.
    pub fn new(
        queue: Arc<dyn PriorityQueue>,
        sessions: Arc<dyn SessionStore>,
        bus: Arc<dyn MessageBus>,
        handler: Arc<dyn AgentHandler>,
        event_sink: Arc<dyn EventSink>,
        concurrency: usize,
        max_retries: u32,
    ) -> Self {
        Self {
            queue,
            sessions,
            bus,
            handler,
            event_sink,
            session_lock: None,
            concurrency,
            max_retries,
            retry_base_delay_ms: 5000,
            session_cancel_tokens: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Return a clone of the shared session cancellation token map so that handlers
    /// (e.g. `WorkspaceHandler`) and commands (e.g. `CancelCommand`) can share it.
    pub fn session_cancel_tokens(&self) -> SessionCancelTokens {
        self.session_cancel_tokens.clone()
    }

    /// Set the base delay for retry backoff (default: 5000ms).
    /// Actual delay = base * 3^retry_count.
    pub fn with_retry_delay(mut self, base_delay_ms: u64) -> Self {
        self.retry_base_delay_ms = base_delay_ms;
        self
    }

    /// Attach a memory store used for distributed session locking.
    ///
    /// When set, each message is processed under a per-session lock to prevent
    /// concurrent workers from corrupting shared conversation history.
    pub fn with_session_lock(mut self, memory: Arc<dyn MemoryStore>) -> Self {
        self.session_lock = Some(memory);
        self
    }

    /// Start workers and process messages until `shutdown` is signalled.
    pub async fn run(&self, shutdown: CancellationToken) -> orka_core::Result<()> {
        info!(concurrency = self.concurrency, "worker pool starting");
        let mut handles = Vec::new();

        for i in 0..self.concurrency {
            let queue = self.queue.clone();
            let sessions = self.sessions.clone();
            let bus = self.bus.clone();
            let handler = self.handler.clone();
            let event_sink = self.event_sink.clone();
            let session_lock = self.session_lock.clone();
            let session_cancel_tokens = self.session_cancel_tokens.clone();
            let cancel = shutdown.clone();
            let max_retries = self.max_retries;
            let retry_base_delay_ms = self.retry_base_delay_ms;

            let handle = tokio::spawn(async move {
                info!(worker = i, "worker started");
                loop {
                    tokio::select! {
                        _ = cancel.cancelled() => {
                            info!(worker = i, "worker shutting down");
                            break;
                        }
                        result = queue.pop(Duration::from_secs(5)) => {
                            match result {
                                Ok(Some(envelope)) => {
                                    // Create a span that ties the entire envelope processing
                                    // to both the session and any incoming trace context.
                                    let trace_id = envelope
                                        .trace_context
                                        .trace_id
                                        .as_deref()
                                        .unwrap_or("none");
                                    let process_span = tracing::info_span!(
                                        "worker.process",
                                        worker = i,
                                        message_id = %envelope.id,
                                        session_id = %envelope.session_id,
                                        trace_id = %trace_id,
                                    );
                                    let _guard = process_span.enter();

                                    // Load session
                                    let session = match sessions.get(&envelope.session_id).await {
                                        Ok(Some(s)) => s,
                                        Ok(None) => {
                                            warn!(worker = i, session_id = %envelope.session_id, "session not found, creating default");
                                            Session::new(&envelope.channel, "anonymous")
                                        }
                                        Err(e) => {
                                            error!(worker = i, %e, "failed to load session");
                                            let retry_count = envelope
                                                .metadata
                                                .get("retry_count")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(0) as u32;
                                            if retry_count < max_retries {
                                                let mut retry_env = envelope.clone();
                                                retry_env.metadata.insert(
                                                    "retry_count".to_string(),
                                                    serde_json::json!(retry_count + 1),
                                                );
                                                let delay_ms = retry_base_delay_ms * 3u64.pow(retry_count);
                                                let not_before = Utc::now() + ChronoDuration::milliseconds(delay_ms as i64);
                                                retry_env.metadata.insert(
                                                    "not_before".to_string(),
                                                    serde_json::json!(not_before.to_rfc3339()),
                                                );
                                                retry_env.priority = Priority::Background;
                                                if let Err(push_err) = queue.push(&retry_env).await {
                                                    error!(worker = i, %push_err, "failed to re-enqueue after session load failure");
                                                }
                                            } else {
                                                // Notify user and drop.
                                                let mut err_env = Envelope::text(
                                                    &envelope.channel,
                                                    envelope.session_id,
                                                    "Temporary error loading session. Please retry.",
                                                );
                                                err_env.trace_context = envelope.trace_context.clone();
                                                if let Err(pub_err) = bus.publish("outbound", &err_env).await {
                                                    error!(worker = i, %pub_err, "failed to publish session error notification");
                                                }
                                            }
                                            continue;
                                        }
                                    };

                                    // `/cancel` bypasses the session lock: cancel the session's token
                                    // immediately and reply without waiting for the ongoing operation.
                                    let is_cancel = matches!(&envelope.payload, Payload::Command(c) if c.name == "cancel");
                                    if is_cancel {
                                        {
                                            let mut tokens = session_cancel_tokens.lock().unwrap();
                                            if let Some(token) = tokens.get(&envelope.session_id) {
                                                token.cancel();
                                            } else {
                                                // No active operation — nothing to cancel.
                                            }
                                            tokens.remove(&envelope.session_id);
                                        }
                                        let mut reply = Envelope::text(
                                            &envelope.channel,
                                            envelope.session_id,
                                            "Cancellation requested. The current operation will stop at the next checkpoint.",
                                        );
                                        reply.metadata = envelope.metadata.clone();
                                        if let Err(e) = bus.publish("outbound", &reply).await {
                                            error!(worker = i, %e, "failed to publish cancel reply");
                                        }
                                        continue;
                                    }

                                    // Register a fresh cancellation token for this session before
                                    // acquiring the lock, so `/cancel` can signal it at any time.
                                    let op_token = {
                                        let token = CancellationToken::new();
                                        session_cancel_tokens
                                            .lock()
                                            .unwrap()
                                            .insert(envelope.session_id, token.clone());
                                        token
                                    };
                                    let _ = op_token; // token is accessed via the map by the handler

                                    // Acquire per-session lock to prevent concurrent history corruption
                                    if let Some(ref lock) = session_lock {
                                        const SESSION_LOCK_TTL_MS: u64 = 120_000;
                                        if !lock.try_acquire_session_lock(&envelope.session_id.to_string(), SESSION_LOCK_TTL_MS).await {
                                            // Another worker is processing this session — re-enqueue with 1s delay
                                            let not_before = Utc::now() + ChronoDuration::milliseconds(1000);
                                            let mut requeue = envelope.clone();
                                            requeue.metadata.insert(
                                                "not_before".to_string(),
                                                serde_json::json!(not_before.to_rfc3339()),
                                            );
                                            if let Err(e) = queue.push(&requeue).await {
                                                error!(worker = i, %e, session_id = %envelope.session_id, "failed to re-enqueue locked session");
                                            } else {
                                                warn!(worker = i, session_id = %envelope.session_id, "session locked by another worker, re-enqueuing with 1s delay");
                                            }
                                            continue;
                                        }
                                    }

                                    // Emit HandlerInvoked
                                    event_sink.emit(DomainEvent::new(
                                        DomainEventKind::HandlerInvoked {
                                            message_id: envelope.id,
                                            session_id: envelope.session_id,
                                        },
                                    )).await;

                                    let start = std::time::Instant::now();
                                    let locked_session_id = envelope.session_id.to_string();

                                    // Handle
                                    match handler.handle(&envelope, &session).await {
                                        Ok(outbound_msgs) => {
                                            let duration_ms = start.elapsed().as_millis() as u64;
                                            let reply_count = outbound_msgs.len();

                                            for msg in &outbound_msgs {
                                                // Wrap outbound as envelope for bus
                                                let mut out_env = Envelope::text(
                                                    &msg.channel,
                                                    msg.session_id,
                                                    match &msg.payload {
                                                        Payload::Text(t) => t.clone(),
                                                        _ => "[non-text]".into(),
                                                    },
                                                );
                                                out_env.metadata = msg.metadata.clone();
                                                out_env.trace_context = envelope.trace_context.clone();
                                                if let Err(e) = bus.publish("outbound", &out_env).await {
                                                    error!(worker = i, %e, "failed to publish outbound");
                                                }
                                            }

                                            // Emit HandlerCompleted
                                            event_sink.emit(DomainEvent::new(
                                                DomainEventKind::HandlerCompleted {
                                                    message_id: envelope.id,
                                                    session_id: envelope.session_id,
                                                    duration_ms,
                                                    reply_count,
                                                },
                                            )).await;

                                            info!(worker = i, message_id = %envelope.id, "processed message");
                                        }
                                        Err(e) => {
                                            // Emit ErrorOccurred
                                            event_sink.emit(DomainEvent::new(
                                                DomainEventKind::ErrorOccurred {
                                                    source: "handler".into(),
                                                    message: e.to_string(),
                                                },
                                            )).await;

                                            let retry_count = envelope
                                                .metadata
                                                .get("retry_count")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(0) as u32;

                                            if retry_count < max_retries {
                                                let mut retry_envelope = envelope.clone();
                                                retry_envelope.metadata.insert(
                                                    "retry_count".to_string(),
                                                    serde_json::json!(retry_count + 1),
                                                );
                                                // Exponential backoff: base * 3^retry_count
                                                let delay_ms = retry_base_delay_ms * 3u64.pow(retry_count);
                                                // Set not_before timestamp so queue skips this until mature
                                                let not_before = Utc::now() + ChronoDuration::milliseconds(delay_ms as i64);
                                                retry_envelope.metadata.insert(
                                                    "not_before".to_string(),
                                                    serde_json::json!(not_before.to_rfc3339()),
                                                );
                                                // Lower priority on each retry
                                                retry_envelope.priority = Priority::Background;
                                                warn!(
                                                    worker = i,
                                                    %e,
                                                    retry_count = retry_count + 1,
                                                    max_retries,
                                                    delay_ms,
                                                    message_id = %envelope.id,
                                                    "handler error, re-enqueuing with not_before delay"
                                                );
                                                if let Err(push_err) = queue.push(&retry_envelope).await {
                                                    error!(worker = i, %push_err, "failed to re-enqueue for retry");
                                                }
                                            } else {
                                                error!(
                                                    worker = i,
                                                    %e,
                                                    retry_count,
                                                    message_id = %envelope.id,
                                                    "max retries exceeded, sending to DLQ"
                                                );
                                                if let Err(dlq_err) = queue.push_dlq(&envelope).await {
                                                    error!(worker = i, %dlq_err, "failed to push to DLQ");
                                                }
                                            }
                                        }
                                    }

                                    // Release session lock after handling (success or error)
                                    if let Some(ref lock) = session_lock {
                                        lock.release_session_lock(&locked_session_id).await;
                                    }

                                    // Clean up the cancellation token for this session.
                                    session_cancel_tokens
                                        .lock()
                                        .unwrap()
                                        .remove(&envelope.session_id);
                                }
                                Ok(None) => {
                                    // Timeout, continue loop
                                }
                                Err(e) => {
                                    error!(worker = i, %e, "queue pop error");
                                }
                            }
                        }
                    }
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            let _ = handle.await;
        }
        info!("worker pool stopped");
        Ok(())
    }
}

/// Concurrent worker pool backed by a `GraphExecutor` + `AgentGraph`.
///
/// Drop-in replacement for [`WorkerPool`] when multi-agent graph execution is desired.
pub struct WorkerPoolGraph {
    queue: Arc<dyn PriorityQueue>,
    sessions: Arc<dyn SessionStore>,
    bus: Arc<dyn MessageBus>,
    executor: Arc<GraphExecutor>,
    graph: Arc<AgentGraph>,
    event_sink: Arc<dyn EventSink>,
    memory: Option<Arc<dyn MemoryStore>>,
    concurrency: usize,
    max_retries: u32,
    retry_base_delay_ms: u64,
}

impl WorkerPoolGraph {
    /// Create a new graph-backed worker pool.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        queue: Arc<dyn PriorityQueue>,
        sessions: Arc<dyn SessionStore>,
        bus: Arc<dyn MessageBus>,
        executor: Arc<GraphExecutor>,
        graph: Arc<AgentGraph>,
        event_sink: Arc<dyn EventSink>,
        concurrency: usize,
        max_retries: u32,
    ) -> Self {
        Self {
            queue,
            sessions,
            bus,
            executor,
            graph,
            event_sink,
            memory: None,
            concurrency,
            max_retries,
            retry_base_delay_ms: 5000,
        }
    }

    /// Set the base delay for retry backoff (default: 5000ms).
    pub fn with_retry_delay(mut self, base_delay_ms: u64) -> Self {
        self.retry_base_delay_ms = base_delay_ms;
        self
    }

    /// Attach a memory store for conversation history persistence.
    ///
    /// When set, the graph pool loads conversation history before each execution
    /// and saves it afterward, enabling multi-turn conversations in the graph path.
    pub fn with_memory(mut self, memory: Arc<dyn MemoryStore>) -> Self {
        self.memory = Some(memory);
        self
    }

    /// Start workers and process messages until `shutdown` is signalled.
    pub async fn run(&self, shutdown: CancellationToken) -> orka_core::Result<()> {
        info!(concurrency = self.concurrency, "graph worker pool starting");
        let mut handles = Vec::new();

        for i in 0..self.concurrency {
            let queue = self.queue.clone();
            let sessions = self.sessions.clone();
            let bus = self.bus.clone();
            let executor = self.executor.clone();
            let graph = self.graph.clone();
            let event_sink = self.event_sink.clone();
            let memory = self.memory.clone();
            let cancel = shutdown.clone();
            let max_retries = self.max_retries;
            let retry_base_delay_ms = self.retry_base_delay_ms;

            let handle = tokio::spawn(async move {
                info!(worker = i, "graph worker started");
                loop {
                    tokio::select! {
                        _ = cancel.cancelled() => {
                            info!(worker = i, "graph worker shutting down");
                            break;
                        }
                        result = queue.pop(Duration::from_secs(5)) => {
                            match result {
                                Ok(Some(envelope)) => {
                                    // Load session
                                    let _session = match sessions.get(&envelope.session_id).await {
                                        Ok(Some(s)) => s,
                                        Ok(None) => {
                                            warn!(worker = i, session_id = %envelope.session_id, "session not found, creating default");
                                            Session::new(&envelope.channel, "anonymous")
                                        }
                                        Err(e) => {
                                            error!(worker = i, %e, "failed to load session");
                                            continue;
                                        }
                                    };

                                    // Acquire per-session lock to prevent concurrent history corruption
                                    if let Some(ref lock) = memory {
                                        const SESSION_LOCK_TTL_MS: u64 = 120_000;
                                        if !lock.try_acquire_session_lock(&envelope.session_id.to_string(), SESSION_LOCK_TTL_MS).await {
                                            let not_before = Utc::now() + ChronoDuration::milliseconds(1000);
                                            let mut requeue = envelope.clone();
                                            requeue.metadata.insert(
                                                "not_before".to_string(),
                                                serde_json::json!(not_before.to_rfc3339()),
                                            );
                                            if let Err(e) = queue.push(&requeue).await {
                                                error!(worker = i, %e, session_id = %envelope.session_id, "failed to re-enqueue locked session");
                                            } else {
                                                warn!(worker = i, session_id = %envelope.session_id, "session locked by another worker, re-enqueuing with 1s delay");
                                            }
                                            continue;
                                        }
                                    }

                                    // Emit HandlerInvoked
                                    event_sink.emit(DomainEvent::new(
                                        DomainEventKind::HandlerInvoked {
                                            message_id: envelope.id,
                                            session_id: envelope.session_id,
                                        },
                                    )).await;

                                    let start = std::time::Instant::now();
                                    let locked_session_id_graph = envelope.session_id.to_string();
                                    let ctx = ExecutionContext::new(envelope.clone());

                                    // Load conversation history from memory store (if available)
                                    // so the graph path has the same multi-turn continuity as WorkspaceHandler.
                                    if let Some(ref mem) = memory {
                                        let history_key = format!("conversation:{}", envelope.session_id);
                                        match mem.recall(&history_key).await {
                                            Ok(Some(entry)) => {
                                                let history: Vec<ChatMessage> =
                                                    serde_json::from_value(entry.value)
                                                        .unwrap_or_default();
                                                if !history.is_empty() {
                                                    ctx.set_messages(history).await;
                                                }
                                            }
                                            Ok(None) => {}
                                            Err(e) => {
                                                warn!(worker = i, %e, session_id = %envelope.session_id, "failed to load conversation history");
                                            }
                                        }
                                    }

                                    // Append the current user message so the graph sees the live input.
                                    let user_text = match &envelope.payload {
                                        Payload::Text(t) => Some(t.clone()),
                                        Payload::Media(m) => m.caption.clone().or_else(|| Some(format!("[media: {}]", m.mime_type))),
                                        Payload::Command(c) => Some(format!("/{}", c.name)),
                                        Payload::Event(_) => None,
                                        _ => None,
                                    };
                                    if let Some(text) = user_text {
                                        ctx.push_message(ChatMessage::user(text)).await;
                                    }

                                    match executor.execute(&graph, &ctx).await {
                                        Ok(result) => {
                                            let duration_ms = start.elapsed().as_millis() as u64;
                                            let outbound_msgs = result.into_outbound_messages(&ctx);
                                            let reply_count = outbound_msgs.len();

                                            // Persist updated conversation history with compaction
                                            if let Some(ref mem) = memory {
                                                let history_key = format!("conversation:{}", envelope.session_id);
                                                let msgs = ctx.messages().await;
                                                history::save_history_compact(mem.as_ref(), &history_key, msgs).await;
                                            }

                                            for msg in &outbound_msgs {
                                                let mut out_env = Envelope::text(
                                                    &msg.channel,
                                                    msg.session_id,
                                                    match &msg.payload {
                                                        Payload::Text(t) => t.clone(),
                                                        _ => "[non-text]".into(),
                                                    },
                                                );
                                                out_env.metadata = msg.metadata.clone();
                                                out_env.trace_context = envelope.trace_context.clone();
                                                if let Err(e) = bus.publish("outbound", &out_env).await {
                                                    error!(worker = i, %e, "failed to publish outbound");
                                                }
                                            }

                                            // Emit HandlerCompleted
                                            event_sink.emit(DomainEvent::new(
                                                DomainEventKind::HandlerCompleted {
                                                    message_id: envelope.id,
                                                    session_id: envelope.session_id,
                                                    duration_ms,
                                                    reply_count,
                                                },
                                            )).await;

                                            info!(worker = i, message_id = %envelope.id, "processed message via graph");
                                        }
                                        Err(e) => {
                                            // Emit ErrorOccurred
                                            event_sink.emit(DomainEvent::new(
                                                DomainEventKind::ErrorOccurred {
                                                    source: "graph_executor".into(),
                                                    message: e.to_string(),
                                                },
                                            )).await;

                                            let retry_count = envelope
                                                .metadata
                                                .get("retry_count")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(0) as u32;

                                            if retry_count < max_retries {
                                                let mut retry_envelope = envelope.clone();
                                                retry_envelope.metadata.insert(
                                                    "retry_count".to_string(),
                                                    serde_json::json!(retry_count + 1),
                                                );
                                                let delay_ms = retry_base_delay_ms * 3u64.pow(retry_count);
                                                let not_before = Utc::now() + ChronoDuration::milliseconds(delay_ms as i64);
                                                retry_envelope.metadata.insert(
                                                    "not_before".to_string(),
                                                    serde_json::json!(not_before.to_rfc3339()),
                                                );
                                                retry_envelope.priority = Priority::Background;
                                                warn!(
                                                    worker = i,
                                                    %e,
                                                    retry_count = retry_count + 1,
                                                    max_retries,
                                                    delay_ms,
                                                    message_id = %envelope.id,
                                                    "graph executor error, re-enqueuing with not_before delay"
                                                );
                                                if let Err(push_err) = queue.push(&retry_envelope).await {
                                                    error!(worker = i, %push_err, "failed to re-enqueue for retry");
                                                }
                                            } else {
                                                error!(
                                                    worker = i,
                                                    %e,
                                                    retry_count,
                                                    message_id = %envelope.id,
                                                    "max retries exceeded, sending to DLQ"
                                                );
                                                if let Err(dlq_err) = queue.push_dlq(&envelope).await {
                                                    error!(worker = i, %dlq_err, "failed to push to DLQ");
                                                }
                                            }
                                        }
                                    }

                                    // Release session lock after graph execution (success or error)
                                    if let Some(ref lock) = memory {
                                        lock.release_session_lock(&locked_session_id_graph).await;
                                    }
                                }
                                Ok(None) => {
                                    // Timeout, continue loop
                                }
                                Err(e) => {
                                    error!(worker = i, %e, "queue pop error");
                                }
                            }
                        }
                    }
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            let _ = handle.await;
        }
        info!("graph worker pool stopped");
        Ok(())
    }
}
