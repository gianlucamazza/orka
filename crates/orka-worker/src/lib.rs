//! Worker pool that consumes messages from the priority queue and dispatches to handlers.
//!
//! - [`WorkerPool`] — concurrent worker loop with retry and dead-letter queue support
//! - [`AgentHandler`] — trait for message handling implementations
//! - [`WorkspaceHandler`] — LLM-powered handler with skill execution and tool loops

#![warn(missing_docs)]

#[allow(missing_docs)]
pub mod commands;
#[allow(missing_docs)]
pub mod handler;
#[allow(missing_docs)]
pub mod stream;
#[allow(missing_docs)]
pub mod workspace_handler;

// re-exports
pub use commands::CommandRegistry;
pub use handler::{AgentHandler, EchoHandler};
pub use stream::{StreamChunk, StreamChunkKind, StreamRegistry};
pub use workspace_handler::{WorkspaceHandler, WorkspaceHandlerConfig};

use std::sync::Arc;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use orka_core::traits::{EventSink, MessageBus, PriorityQueue, SessionStore};
use orka_core::{DomainEvent, DomainEventKind, Envelope, Payload, Priority, Session};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Concurrent worker pool that pops envelopes from a priority queue and dispatches them.
pub struct WorkerPool {
    queue: Arc<dyn PriorityQueue>,
    sessions: Arc<dyn SessionStore>,
    bus: Arc<dyn MessageBus>,
    handler: Arc<dyn AgentHandler>,
    event_sink: Arc<dyn EventSink>,
    concurrency: usize,
    max_retries: u32,
    retry_base_delay_ms: u64,
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
            concurrency,
            max_retries,
            retry_base_delay_ms: 5000,
        }
    }

    /// Set the base delay for retry backoff (default: 5000ms).
    /// Actual delay = base * 3^retry_count.
    pub fn with_retry_delay(mut self, base_delay_ms: u64) -> Self {
        self.retry_base_delay_ms = base_delay_ms;
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
                                    // Load session
                                    let session = match sessions.get(&envelope.session_id).await {
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

                                    // Emit HandlerInvoked
                                    event_sink.emit(DomainEvent::new(
                                        DomainEventKind::HandlerInvoked {
                                            message_id: envelope.id,
                                            session_id: envelope.session_id,
                                        },
                                    )).await;

                                    let start = std::time::Instant::now();

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
