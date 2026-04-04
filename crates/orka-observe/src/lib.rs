//! Observability infrastructure: event sinks, metrics, and OpenTelemetry
//! integration.
//!
//! - [`create_event_sink`] — factory that selects the appropriate [`EventSink`]
//!   backend
//! - [`metrics`] — Prometheus-compatible metrics collection
//! - [`otel_sink`] — OpenTelemetry trace/span export
//! - [`audit_sink`] — Append-only skill invocation audit log

#![warn(missing_docs)]

use std::sync::Arc;

use async_trait::async_trait;
use orka_core::{DomainEvent, DomainEventKind, traits::EventSink};
use tracing::{debug, info, warn};

/// Append-only JSONL audit log for skill invocations.
pub mod audit_sink;
/// Observability configuration owned by `orka-observe`.
pub mod config;
/// Prometheus-compatible counter and histogram metrics.
pub mod metrics;
/// OpenTelemetry OTLP span exporter event sink.
pub mod otel_sink;
/// Redis Streams event sink with batched writes.
pub mod redis_sink;

pub use crate::config::{AuditConfig, ObserveConfig};

struct LogEventSink;

#[async_trait]
#[allow(clippy::too_many_lines)]
impl EventSink for LogEventSink {
    async fn emit(&self, event: DomainEvent) {
        metrics::record_event(&event);
        match &event.kind {
            DomainEventKind::MessageReceived {
                message_id,
                channel,
                session_id,
            } => {
                info!(%message_id, %channel, %session_id, "message received");
            }
            DomainEventKind::SessionCreated {
                session_id,
                channel,
            } => {
                info!(%session_id, %channel, "session created");
            }
            DomainEventKind::HandlerInvoked {
                message_id,
                session_id,
            } => {
                info!(%message_id, %session_id, "handler invoked");
            }
            DomainEventKind::HandlerCompleted {
                message_id,
                session_id,
                duration_ms,
                reply_count,
            } => {
                info!(%message_id, %session_id, duration_ms, reply_count, "handler completed");
            }
            DomainEventKind::SkillInvoked {
                skill_name,
                message_id,
                caller_id,
                ..
            } => {
                info!(skill_name, %message_id, caller_id = caller_id.as_deref().unwrap_or("-"), "skill invoked");
            }
            DomainEventKind::SkillCompleted {
                skill_name,
                message_id,
                duration_ms,
                success,
                error_message,
                ..
            } => {
                if let Some(err) = error_message {
                    info!(skill_name, %message_id, duration_ms, success, error = err, "skill completed");
                } else {
                    info!(skill_name, %message_id, duration_ms, success, "skill completed");
                }
            }
            DomainEventKind::LlmRequest {
                message_id,
                model,
                provider,
                iteration,
            } => {
                debug!(%message_id, model, provider, iteration, "llm request");
            }
            DomainEventKind::LlmCompleted {
                message_id,
                model,
                provider,
                input_tokens,
                output_tokens,
                reasoning_tokens,
                duration_ms,
                estimated_cost_usd,
            } => {
                if let Some(cost) = estimated_cost_usd {
                    info!(%message_id, model, provider, input_tokens, output_tokens, reasoning_tokens, duration_ms, cost, "llm completed");
                } else {
                    info!(%message_id, model, provider, input_tokens, output_tokens, reasoning_tokens, duration_ms, "llm completed");
                }
            }
            DomainEventKind::ErrorOccurred { source, message } => {
                warn!(source, message, "error occurred");
            }
            DomainEventKind::AgentReasoning {
                message_id,
                iteration,
                reasoning_text,
            } => {
                debug!(%message_id, iteration, reasoning_len = reasoning_text.len(), "agent reasoning extracted");
            }
            DomainEventKind::AgentIteration {
                message_id,
                iteration,
                tool_count,
                tokens_used,
                elapsed_ms,
            } => {
                info!(%message_id, iteration, tool_count, tokens_used, elapsed_ms, "agent iteration completed");
            }
            DomainEventKind::PrivilegedCommandExecuted {
                message_id,
                session_id,
                command,
                args,
                success,
                duration_ms,
                ..
            } => {
                warn!(
                    %message_id, %session_id, command,
                    args = ?args, success, duration_ms,
                    "privileged command executed"
                );
            }
            DomainEventKind::PrivilegedCommandDenied {
                message_id,
                session_id,
                command,
                args,
                reason,
            } => {
                warn!(
                    %message_id, %session_id, command,
                    args = ?args, reason,
                    "privileged command denied"
                );
            }
            DomainEventKind::PrinciplesInjected { session_id, count } => {
                info!(%session_id, count, "principles injected into prompt");
            }
            DomainEventKind::ReflectionCompleted {
                session_id,
                principles_created,
                trajectory_id,
            } => {
                info!(%session_id, principles_created, trajectory_id, "reflection completed");
            }
            DomainEventKind::TrajectoryRecorded {
                session_id,
                trajectory_id,
            } => {
                info!(%session_id, trajectory_id, "trajectory recorded");
            }
            DomainEventKind::DistillationCompleted {
                workspace,
                principles_created,
            } => {
                info!(workspace, principles_created, "distillation completed");
            }
            DomainEventKind::SkillDisabled {
                skill_name,
                reason,
                source,
            } => {
                warn!(skill_name, reason, source, "skill disabled");
            }
            DomainEventKind::Heartbeat => {
                debug!("heartbeat");
            }
            DomainEventKind::ScheduleTriggered {
                schedule_name,
                skill_name,
                ..
            } => {
                info!(
                    schedule_name,
                    skill = skill_name.as_deref().unwrap_or("-"),
                    "schedule triggered"
                );
            }
            DomainEventKind::AgentDelegated {
                run_id,
                source_agent,
                target_agent,
                mode,
                reason,
            } => {
                info!(
                    run_id,
                    source_agent, target_agent, mode, reason, "agent delegated"
                );
            }
            DomainEventKind::AgentCompleted {
                run_id,
                agent_id,
                iterations,
                tokens,
                duration_ms,
                success,
            } => {
                info!(
                    run_id,
                    agent_id, iterations, tokens, duration_ms, success, "agent completed"
                );
            }
            DomainEventKind::RunInterrupted {
                run_id,
                agent_id,
                tool_name,
            } => {
                info!(
                    run_id,
                    agent_id, tool_name, "run interrupted — awaiting approval"
                );
            }
            DomainEventKind::GraphCompleted {
                run_id,
                graph_id,
                agents_executed,
                total_iterations,
                total_tokens,
                duration_ms,
            } => {
                info!(
                    run_id,
                    graph_id,
                    agents = agents_executed.len(),
                    total_iterations,
                    total_tokens,
                    duration_ms,
                    "graph completed"
                );
            }
            _ => {
                debug!(kind = ?event.kind, "unhandled domain event");
            }
        }
    }
}

/// Create an [`EventSink`] from the given configuration.
///
/// If `audit.enabled` is true, the primary sink is wrapped in a
/// [`FanoutSink`] that also writes to the [`audit_sink::AuditSink`].
pub fn create_event_sink(
    observe: &ObserveConfig,
    audit: &AuditConfig,
    redis_url: &str,
) -> Arc<dyn EventSink> {
    let primary: Arc<dyn EventSink> = match observe.backend.as_str() {
        "redis" => match redis_sink::RedisEventSink::new(
            redis_url,
            observe.batch_size,
            observe.flush_interval_ms,
        ) {
            Ok(sink) => {
                info!("event sink: Redis Streams");
                Arc::new(sink)
            }
            Err(e) => {
                warn!(%e, "failed to create Redis event sink, falling back to log");
                Arc::new(LogEventSink)
            }
        },
        "otel" | "otlp" => match otel_sink::init_otel_tracer("orka") {
            Ok(tracer) => {
                info!("event sink: OpenTelemetry (OTLP)");
                Arc::new(otel_sink::OtelEventSink::new(tracer))
            }
            Err(e) => {
                warn!(%e, "failed to initialize OTel, falling back to log");
                Arc::new(LogEventSink)
            }
        },
        _ => {
            info!("event sink: log");
            Arc::new(LogEventSink)
        }
    };

    if audit.enabled {
        let path_str = audit
            .path
            .as_deref()
            .map_or_else(|| "orka-audit.jsonl".into(), |p| p.to_string_lossy());
        match audit_sink::AuditSink::new(path_str.as_ref()) {
            Ok(audit) => {
                info!(path = path_str.as_ref(), "audit log enabled");
                Arc::new(FanoutSink(vec![primary, Arc::new(audit)]))
            }
            Err(e) => {
                warn!(%e, "failed to open audit log, audit disabled");
                primary
            }
        }
    } else {
        primary
    }
}

/// Broadcasts events to multiple sinks in sequence.
struct FanoutSink(Vec<Arc<dyn EventSink>>);

#[async_trait]
impl EventSink for FanoutSink {
    async fn emit(&self, event: DomainEvent) {
        for sink in &self.0 {
            sink.emit(event.clone()).await;
        }
    }
}

#[cfg(test)]
#[allow(clippy::default_trait_access, clippy::too_many_lines)]
pub(crate) mod test_helpers {
    use orka_core::{
        DomainEvent, DomainEventKind,
        types::{MessageId, SessionId},
    };

    pub(crate) fn make_event(kind: DomainEventKind) -> DomainEvent {
        DomainEvent::new(kind)
    }

    pub(crate) fn all_event_kinds() -> Vec<DomainEventKind> {
        let mid = MessageId::new();
        let sid = SessionId::new();
        vec![
            DomainEventKind::MessageReceived {
                message_id: mid,
                channel: "test".into(),
                session_id: sid,
            },
            DomainEventKind::SessionCreated {
                session_id: sid,
                channel: "test".into(),
            },
            DomainEventKind::HandlerInvoked {
                message_id: mid,
                session_id: sid,
            },
            DomainEventKind::HandlerCompleted {
                message_id: mid,
                session_id: sid,
                duration_ms: 42,
                reply_count: 1,
            },
            DomainEventKind::SkillInvoked {
                skill_name: "echo".into(),
                message_id: mid,
                input_args: Default::default(),
                caller_id: None,
            },
            DomainEventKind::SkillCompleted {
                skill_name: "echo".into(),
                message_id: mid,
                duration_ms: 10,
                success: true,
                error_category: None,
                output_preview: None,
                error_message: None,
            },
            DomainEventKind::LlmRequest {
                message_id: mid,
                model: "claude-sonnet-4-6".into(),
                provider: "anthropic".into(),
                iteration: 1,
            },
            DomainEventKind::LlmCompleted {
                message_id: mid,
                model: "gpt-test".into(),
                provider: "openai".into(),
                input_tokens: 100,
                output_tokens: 50,
                reasoning_tokens: 0,
                duration_ms: 200,
                estimated_cost_usd: Some(0.005),
            },
            DomainEventKind::ErrorOccurred {
                source: "test".into(),
                message: "boom".into(),
            },
            DomainEventKind::AgentReasoning {
                message_id: mid,
                iteration: 0,
                reasoning_text: "Let me think...".into(),
            },
            DomainEventKind::AgentIteration {
                message_id: mid,
                iteration: 0,
                tool_count: 2,
                tokens_used: 500,
                elapsed_ms: 1200,
            },
            DomainEventKind::PrivilegedCommandExecuted {
                message_id: mid,
                session_id: sid,
                command: "systemctl".into(),
                args: vec!["restart".into(), "nginx".into()],
                approval_id: None,
                approved_by: None,
                exit_code: Some(0),
                success: true,
                duration_ms: 150,
            },
            DomainEventKind::PrivilegedCommandDenied {
                message_id: mid,
                session_id: sid,
                command: "rm".into(),
                args: vec!["-rf".into(), "/".into()],
                reason: "blocked".into(),
            },
            DomainEventKind::PrinciplesInjected {
                session_id: sid,
                count: 3,
            },
            DomainEventKind::ReflectionCompleted {
                session_id: sid,
                principles_created: 2,
                trajectory_id: "traj-1".into(),
            },
            DomainEventKind::TrajectoryRecorded {
                session_id: sid,
                trajectory_id: "traj-1".into(),
            },
            DomainEventKind::DistillationCompleted {
                workspace: "default".into(),
                principles_created: 3,
            },
            DomainEventKind::SkillDisabled {
                skill_name: "package_updates".into(),
                reason: "checkupdates crashed".into(),
                source: "experience_feedback".into(),
            },
            DomainEventKind::Heartbeat,
            DomainEventKind::ScheduleTriggered {
                schedule_name: "daily-digest".into(),
                workspace: Some("default".into()),
                skill_name: Some("send_digest".into()),
            },
            DomainEventKind::AgentDelegated {
                run_id: "run-1".into(),
                source_agent: "planner".into(),
                target_agent: "executor".into(),
                mode: "handoff".into(),
                reason: "subtask".into(),
            },
            DomainEventKind::AgentCompleted {
                run_id: "run-1".into(),
                agent_id: "executor".into(),
                iterations: 3,
                tokens: 1500,
                duration_ms: 2500,
                success: true,
            },
            DomainEventKind::RunInterrupted {
                run_id: "run-1".into(),
                agent_id: "executor".into(),
                tool_name: "delete_file".into(),
            },
            DomainEventKind::GraphCompleted {
                run_id: "run-1".into(),
                graph_id: "graph-a".into(),
                agents_executed: vec!["planner".into(), "executor".into()],
                total_iterations: 5,
                total_tokens: 3000,
                duration_ms: 5000,
            },
        ]
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default,
    clippy::default_trait_access,
    clippy::needless_pass_by_value,
    clippy::stable_sort_primitive
)]
mod tests {
    use std::sync::{Arc, Mutex};

    use orka_core::DomainEvent;

    use super::{test_helpers::*, *};

    fn test_config() -> (ObserveConfig, AuditConfig, String) {
        (
            ObserveConfig::default(),
            AuditConfig::default(),
            "redis://127.0.0.1:6379".into(),
        )
    }

    /// Recording sink that collects emitted events for assertions.
    struct RecordingEventSink {
        events: Arc<Mutex<Vec<DomainEvent>>>,
    }

    impl RecordingEventSink {
        fn new() -> (Self, Arc<Mutex<Vec<DomainEvent>>>) {
            let store = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    events: store.clone(),
                },
                store,
            )
        }
    }

    #[async_trait::async_trait]
    impl orka_core::traits::EventSink for RecordingEventSink {
        async fn emit(&self, event: DomainEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[tokio::test]
    async fn log_event_sink_emits_all_variants_without_panic() {
        let sink = LogEventSink;
        for kind in all_event_kinds() {
            sink.emit(make_event(kind)).await;
        }
    }

    #[test]
    fn create_event_sink_returns_log_for_unknown_backend() {
        let (mut observe, audit, redis_url) = test_config();
        observe.backend = "unknown".into();
        let _sink = create_event_sink(&observe, &audit, &redis_url);
    }

    #[test]
    fn create_event_sink_falls_back_to_log_for_invalid_redis_url() {
        let (mut observe, audit, _) = test_config();
        observe.backend = "redis".into();
        let _sink = create_event_sink(&observe, &audit, "not-a-valid-url");
    }

    #[tokio::test]
    async fn fanout_broadcasts_to_all_sinks() {
        let (sink_a, store_a) = RecordingEventSink::new();
        let (sink_b, store_b) = RecordingEventSink::new();
        let fanout = FanoutSink(vec![Arc::new(sink_a), Arc::new(sink_b)]);

        fanout
            .emit(make_event(orka_core::DomainEventKind::Heartbeat))
            .await;
        fanout
            .emit(make_event(orka_core::DomainEventKind::Heartbeat))
            .await;

        assert_eq!(store_a.lock().unwrap().len(), 2);
        assert_eq!(store_b.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn create_event_sink_with_audit_writes_to_file() {
        use orka_core::{DomainEventKind, types::MessageId};
        use tempfile::NamedTempFile;

        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        let (observe, mut audit, redis_url) = test_config();
        audit.enabled = true;
        audit.path = Some(path.clone());

        let sink = create_event_sink(&observe, &audit, &redis_url);

        // Emit a SkillInvoked event — the audit sink only writes skill events
        sink.emit(DomainEvent::new(DomainEventKind::SkillInvoked {
            skill_name: "test_skill".into(),
            message_id: MessageId::new(),
            input_args: Default::default(),
            caller_id: None,
        }))
        .await;

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            !content.is_empty(),
            "audit file should contain a JSONL record"
        );
        let record: serde_json::Value =
            serde_json::from_str(content.lines().next().unwrap()).unwrap();
        assert_eq!(record["skill"], "test_skill");
    }
}
