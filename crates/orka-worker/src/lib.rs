//! Worker pool that consumes messages from the priority queue and dispatches to
//! handlers.
//!
//! - [`WorkerPool`] — concurrent worker loop with retry, DLQ, tracing, and
//!   session-lock support
//! - [`Dispatcher`] — pluggable dispatch strategy
//! - [`HandlerDispatcher`] — wraps an [`AgentHandler`] (tests / simple use)
//! - [`GraphDispatcher`] — drives the full agent graph
//! - [`AgentHandler`] — trait for simple message handling implementations
//! - [`WorkspaceHandler`] — LLM-powered handler with skill execution and tool
//!   loops

#![warn(missing_docs)]

/// Slash command framework and built-in command implementations.
pub mod commands;
/// Pluggable dispatch strategies ([`HandlerDispatcher`], [`GraphDispatcher`]).
pub mod dispatcher;
/// [`AgentHandler`] trait and the built-in [`EchoHandler`].
pub mod handler;
/// Shared conversation-history helpers (compaction + persistence).
pub mod history;
/// Re-exports streaming types from `orka-core`.
pub mod stream;
/// LLM-powered agent handler with tool loops and guardrails.
pub mod workspace_handler;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::{Duration as ChronoDuration, Utc};
pub use commands::CommandRegistry;
pub use dispatcher::{Dispatcher, GraphDispatcher, HandlerDispatcher};
pub use handler::{AgentHandler, EchoHandler};
use orka_core::{
    DomainEvent, DomainEventKind, Envelope, OutboundMessage, Payload, Priority, Session,
    traits::{DeadLetterQueue, EventSink, MessageBus, PriorityQueue, SessionLock, SessionStore},
    types::SessionId,
};
pub use stream::{StreamChunk, StreamChunkKind, StreamRegistry};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
pub use workspace_handler::{WorkspaceHandler, WorkspaceHandlerConfig};

/// Initial TTL for per-session distributed locks.
///
/// Short to limit deadlock duration on worker crash. The watchdog task renews
/// the lock every [`SESSION_LOCK_RENEWAL_MS`] while a dispatch is in progress.
const SESSION_LOCK_TTL_MS: u64 = 30_000;

/// Watchdog renewal interval — TTL / 3 to guarantee renewal before expiry.
const SESSION_LOCK_RENEWAL_MS: u64 = SESSION_LOCK_TTL_MS / 3;

/// Maximum lock renewals before the watchdog gives up (~10 min total).
const SESSION_LOCK_MAX_RENEWALS: u32 = 60;

/// Shared map of per-session cancellation tokens.
///
/// The worker registers a token before each dispatch; the `/cancel` command
/// uses it to abort ongoing LLM loops.
pub type SessionCancelTokens = Arc<Mutex<HashMap<SessionId, CancellationToken>>>;

/// Concurrent worker pool that pops envelopes from a priority queue and
/// dispatches them via a pluggable [`Dispatcher`].
pub struct WorkerPool {
    queue: Arc<dyn PriorityQueue>,
    sessions: Arc<dyn SessionStore>,
    bus: Arc<dyn MessageBus>,
    dispatcher: Arc<dyn Dispatcher>,
    event_sink: Arc<dyn EventSink>,
    session_lock: Option<Arc<dyn SessionLock>>,
    dlq: Option<Arc<dyn DeadLetterQueue>>,
    concurrency: usize,
    max_retries: u32,
    retry_base_delay_ms: u64,
    session_cancel_tokens: SessionCancelTokens,
}

impl WorkerPool {
    /// Create a new pool with the given concurrency level and retry policy.
    pub fn new(
        queue: Arc<dyn PriorityQueue>,
        sessions: Arc<dyn SessionStore>,
        bus: Arc<dyn MessageBus>,
        dispatcher: Arc<dyn Dispatcher>,
        event_sink: Arc<dyn EventSink>,
        concurrency: usize,
        max_retries: u32,
    ) -> Self {
        Self {
            queue,
            sessions,
            bus,
            dispatcher,
            event_sink,
            session_lock: None,
            dlq: None,
            concurrency,
            max_retries,
            retry_base_delay_ms: 5000,
            session_cancel_tokens: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Return a clone of the shared session cancellation token map so that
    /// handlers (e.g. [`WorkspaceHandler`]) and commands (e.g.
    /// `CancelCommand`) can share it.
    pub fn session_cancel_tokens(&self) -> SessionCancelTokens {
        self.session_cancel_tokens.clone()
    }

    /// Set the base delay for retry backoff (default: 5000 ms).
    /// Actual delay = `base * 3^retry_count`.
    pub fn with_retry_delay(mut self, base_delay_ms: u64) -> Self {
        self.retry_base_delay_ms = base_delay_ms;
        self
    }

    /// Attach a session lock for distributed per-session locking.
    ///
    /// When set, each message is processed under a per-session lock to prevent
    /// concurrent workers from corrupting shared conversation history.
    pub fn with_session_lock(mut self, lock: Arc<dyn SessionLock>) -> Self {
        self.session_lock = Some(lock);
        self
    }

    /// Attach a dead-letter queue for messages that exhaust all retry attempts.
    pub fn with_dlq(mut self, dlq: Arc<dyn DeadLetterQueue>) -> Self {
        self.dlq = Some(dlq);
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
            let dispatcher = self.dispatcher.clone();
            let event_sink = self.event_sink.clone();
            let session_lock = self.session_lock.clone();
            let dlq = self.dlq.clone();
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
                                    // 1. Tracing span
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

                                    // 2. Load session
                                    let session = match sessions.get(&envelope.session_id).await {
                                        Ok(Some(s)) => s,
                                        Ok(None) => {
                                            warn!(
                                                worker = i,
                                                session_id = %envelope.session_id,
                                                "session not found, creating default"
                                            );
                                            let mut s = Session::new(&envelope.channel, "anonymous");
                                            s.id = envelope.session_id;
                                            if let Err(e) = sessions.put(&s).await {
                                                warn!(worker = i, %e, "failed to persist fallback session");
                                            }
                                            s
                                        }
                                        Err(e) => {
                                            error!(worker = i, %e, "failed to load session");
                                            let retry_count = envelope
                                                .metadata
                                                .get("retry_count")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(0) as u32;
                                            retry_or_dlq(
                                                &queue,
                                                dlq.as_deref(),
                                                &envelope,
                                                retry_count,
                                                max_retries,
                                                retry_base_delay_ms,
                                            )
                                            .await;
                                            continue;
                                        }
                                    };

                                    // 3. Cancel fast-path: respond immediately without locking
                                    let is_cancel = matches!(
                                        &envelope.payload,
                                        Payload::Command(c) if c.name == "cancel"
                                    );
                                    if is_cancel {
                                        if let Ok(mut tokens) = session_cancel_tokens.lock() {
                                            if let Some(token) = tokens.get(&envelope.session_id) {
                                                token.cancel();
                                            }
                                            tokens.remove(&envelope.session_id);
                                        } else {
                                            error!(worker = i, "cancel tokens lock poisoned");
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

                                    // 4. Register cancel token before acquiring lock
                                    let op_token = {
                                        let token = CancellationToken::new();
                                        if let Ok(mut tokens) = session_cancel_tokens.lock() {
                                            tokens.insert(envelope.session_id, token.clone());
                                        } else {
                                            error!(worker = i, "cancel tokens lock poisoned");
                                        }
                                        token
                                    };
                                    let _ = op_token; // accessed via map by handler

                                    // 5. Acquire session lock
                                    if let Some(ref lock) = session_lock
                                        && !lock
                                            .try_acquire(
                                                &envelope.session_id.to_string(),
                                                SESSION_LOCK_TTL_MS,
                                            )
                                            .await
                                    {
                                        let not_before =
                                            Utc::now() + ChronoDuration::milliseconds(1000);
                                        let mut requeue = envelope.clone();
                                        requeue.metadata.insert(
                                            "not_before".to_string(),
                                            serde_json::json!(not_before.to_rfc3339()),
                                        );
                                        if let Err(e) = queue.push(&requeue).await {
                                            error!(
                                                worker = i,
                                                %e,
                                                session_id = %envelope.session_id,
                                                "failed to re-enqueue locked session"
                                            );
                                        } else {
                                            warn!(
                                                worker = i,
                                                session_id = %envelope.session_id,
                                                "session locked by another worker, re-enqueuing with 1s delay"
                                            );
                                        }
                                        // Clean up the cancel token we registered
                                        if let Ok(mut tokens) = session_cancel_tokens.lock() {
                                            tokens.remove(&envelope.session_id);
                                        }
                                        continue;
                                    }

                                    // 6. Emit HandlerInvoked
                                    event_sink
                                        .emit(DomainEvent::new(DomainEventKind::HandlerInvoked {
                                            message_id: envelope.id,
                                            session_id: envelope.session_id,
                                        }))
                                        .await;

                                    let start = std::time::Instant::now();
                                    let locked_session_id = envelope.session_id.to_string();

                                    // 6a. Watchdog: renew the session lock periodically so
                                    // multi-iteration dispatches (>TTL) never lose the lock.
                                    // Aborted immediately after dispatch completes.
                                    let watchdog = session_lock.as_ref().map(|lock| {
                                        let lock = Arc::clone(lock);
                                        let sid = locked_session_id.clone();
                                        tokio::spawn(async move {
                                            let mut interval = tokio::time::interval(
                                                Duration::from_millis(SESSION_LOCK_RENEWAL_MS),
                                            );
                                            interval.tick().await;
                                            let mut renewals: u32 = 0;
                                            loop {
                                                interval.tick().await;
                                                if renewals >= SESSION_LOCK_MAX_RENEWALS {
                                                    warn!(
                                                        session_id = %sid,
                                                        "session lock watchdog hit max renewals"
                                                    );
                                                    break;
                                                }
                                                if !lock.extend(&sid, SESSION_LOCK_TTL_MS).await {
                                                    warn!(
                                                        session_id = %sid,
                                                        "session lock expired before renewal"
                                                    );
                                                    break;
                                                }
                                                renewals += 1;
                                            }
                                        })
                                    });

                                    // 7. Dispatch
                                    match dispatcher.dispatch(&envelope, &session).await {
                                        Ok(outbound_msgs) => {
                                            // 8. Publish outbound + emit HandlerCompleted
                                            let duration_ms =
                                                start.elapsed().as_millis() as u64;
                                            let reply_count = outbound_msgs.len();
                                            publish_outbound(&bus, &envelope, &outbound_msgs)
                                                .await;
                                            event_sink
                                                .emit(DomainEvent::new(
                                                    DomainEventKind::HandlerCompleted {
                                                        message_id: envelope.id,
                                                        session_id: envelope.session_id,
                                                        duration_ms,
                                                        reply_count,
                                                    },
                                                ))
                                                .await;
                                            info!(
                                                worker = i,
                                                message_id = %envelope.id,
                                                "processed message"
                                            );
                                        }
                                        Err(e) => {
                                            // 9. Retry or DLQ
                                            event_sink
                                                .emit(DomainEvent::new(
                                                    DomainEventKind::ErrorOccurred {
                                                        source: "dispatcher".into(),
                                                        message: e.to_string(),
                                                    },
                                                ))
                                                .await;
                                            let retry_count = envelope
                                                .metadata
                                                .get("retry_count")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(0)
                                                as u32;
                                            warn!(
                                                worker = i,
                                                %e,
                                                message_id = %envelope.id,
                                                "dispatcher error"
                                            );
                                            retry_or_dlq(
                                                &queue,
                                                dlq.as_deref(),
                                                &envelope,
                                                retry_count,
                                                max_retries,
                                                retry_base_delay_ms,
                                            )
                                            .await;
                                        }
                                    }

                                    // 10. Stop watchdog, then release session lock
                                    if let Some(wdog) = watchdog {
                                        wdog.abort();
                                    }
                                    if let Some(ref lock) = session_lock {
                                        lock.release(&locked_session_id).await;
                                    }

                                    // 11. Remove cancel token
                                    if let Ok(mut tokens) = session_cancel_tokens.lock() {
                                        tokens.remove(&envelope.session_id);
                                    } else {
                                        error!(worker = i, "cancel tokens lock poisoned");
                                    }
                                }
                                Ok(None) => {
                                    // Pop timeout — continue loop
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

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Publish outbound messages to the `"outbound"` bus topic, propagating trace
/// context from the source envelope.
async fn publish_outbound(bus: &Arc<dyn MessageBus>, source: &Envelope, msgs: &[OutboundMessage]) {
    for msg in msgs {
        let mut out_env = Envelope::text(
            &msg.channel,
            msg.session_id,
            match &msg.payload {
                Payload::Text(t) => t.clone(),
                _ => "[non-text]".into(),
            },
        );
        out_env.metadata = msg.metadata.clone();
        out_env.trace_context = source.trace_context.clone();
        if let Err(e) = bus.publish("outbound", &out_env).await {
            error!(%e, "failed to publish outbound");
        }
    }
}

/// Retry with exponential backoff, or push to the DLQ when retries are
/// exhausted.
async fn retry_or_dlq(
    queue: &Arc<dyn PriorityQueue>,
    dlq: Option<&dyn DeadLetterQueue>,
    envelope: &Envelope,
    retry_count: u32,
    max_retries: u32,
    base_delay_ms: u64,
) {
    if retry_count < max_retries {
        let mut retry_envelope = envelope.clone();
        retry_envelope.metadata.insert(
            "retry_count".to_string(),
            serde_json::json!(retry_count + 1),
        );
        let delay_ms = base_delay_ms * 3u64.pow(retry_count);
        let not_before = Utc::now() + ChronoDuration::milliseconds(delay_ms as i64);
        retry_envelope.metadata.insert(
            "not_before".to_string(),
            serde_json::json!(not_before.to_rfc3339()),
        );
        retry_envelope.priority = Priority::Background;
        warn!(
            retry_count = retry_count + 1,
            max_retries,
            delay_ms,
            message_id = %envelope.id,
            "error, re-enqueuing with not_before delay"
        );
        if let Err(push_err) = queue.push(&retry_envelope).await {
            error!(%push_err, "failed to re-enqueue for retry");
        }
    } else {
        error!(
            retry_count,
            message_id = %envelope.id,
            "max retries exceeded, sending to DLQ"
        );
        if let Some(dlq) = dlq
            && let Err(dlq_err) = dlq.push(envelope).await
        {
            error!(%dlq_err, "failed to push to DLQ");
        }
    }
}
