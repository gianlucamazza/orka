//! Observability infrastructure: event sinks, metrics, and OpenTelemetry integration.
//!
//! - [`create_event_sink`] — factory that selects the appropriate [`EventSink`] backend
//! - [`metrics`] — Prometheus-compatible metrics collection
//! - [`otel_sink`] — OpenTelemetry trace/span export

#![warn(missing_docs)]

use std::sync::Arc;

use async_trait::async_trait;
use orka_core::config::OrkaConfig;
use orka_core::traits::EventSink;
use orka_core::{DomainEvent, DomainEventKind};
use tracing::{debug, info, warn};

#[allow(missing_docs)]
pub mod metrics;
#[allow(missing_docs)]
pub mod otel_sink;
#[allow(missing_docs)]
pub mod redis_sink;

struct LogEventSink;

#[async_trait]
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
            } => {
                info!(skill_name, %message_id, "skill invoked");
            }
            DomainEventKind::SkillCompleted {
                skill_name,
                message_id,
                duration_ms,
                success,
            } => {
                info!(skill_name, %message_id, duration_ms, success, "skill completed");
            }
            DomainEventKind::LlmCompleted {
                message_id,
                model,
                input_tokens,
                output_tokens,
                duration_ms,
                estimated_cost_usd,
            } => {
                if let Some(cost) = estimated_cost_usd {
                    info!(%message_id, model, input_tokens, output_tokens, duration_ms, cost, "llm completed");
                } else {
                    info!(%message_id, model, input_tokens, output_tokens, duration_ms, "llm completed");
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
            DomainEventKind::Heartbeat => {
                debug!("heartbeat");
            }
            _ => {
                debug!(kind = ?event.kind, "unhandled domain event");
            }
        }
    }
}

/// Create an [`EventSink`] from the given configuration.
pub fn create_event_sink(config: &OrkaConfig) -> Arc<dyn EventSink> {
    match config.observe.backend.as_str() {
        "redis" => match redis_sink::RedisEventSink::new(
            &config.redis.url,
            config.observe.batch_size,
            config.observe.flush_interval_ms,
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orka_core::config::*;
    use orka_core::types::{MessageId, SessionId};

    fn test_config() -> OrkaConfig {
        OrkaConfig {
            config_version: 1,
            server: ServerConfig::default(),
            bus: BusConfig::default(),
            redis: RedisConfig::default(),
            logging: LoggingConfig::default(),
            workspace_dir: ".".into(),
            workspaces: Vec::new(),
            default_workspace: None,
            adapters: AdapterConfig::default(),
            worker: WorkerConfig::default(),
            memory: MemoryConfig::default(),
            secrets: SecretConfig::default(),
            auth: AuthConfig::default(),
            sandbox: SandboxConfig::default(),
            plugins: PluginConfig::default(),
            session: SessionConfig::default(),
            queue: QueueConfig::default(),
            llm: LlmConfig::default(),
            agent: AgentConfig::default(),
            tools: ToolsConfig::default(),
            observe: ObserveConfig::default(),
            gateway: GatewayConfig::default(),
            mcp: McpConfig::default(),
            guardrails: GuardrailsConfig::default(),
            web: WebConfig::default(),
            os: OsConfig::default(),
            a2a: A2aConfig::default(),
            knowledge: KnowledgeConfig::default(),
            scheduler: SchedulerConfig::default(),
            http: HttpClientConfig::default(),
            experience: ExperienceConfig::default(),
        }
    }

    fn make_event(kind: DomainEventKind) -> DomainEvent {
        DomainEvent::new(kind)
    }

    fn all_event_kinds() -> Vec<DomainEventKind> {
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
            },
            DomainEventKind::SkillCompleted {
                skill_name: "echo".into(),
                message_id: mid,
                duration_ms: 10,
                success: true,
            },
            DomainEventKind::LlmCompleted {
                message_id: mid,
                model: "gpt-test".into(),
                input_tokens: 100,
                output_tokens: 50,
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
            DomainEventKind::Heartbeat,
        ]
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
        let mut config = test_config();
        config.observe.backend = "unknown".into();
        // Should return a sink (the log fallback) without error.
        let _sink = create_event_sink(&config);
    }

    #[test]
    fn create_event_sink_falls_back_to_log_for_invalid_redis_url() {
        let mut config = test_config();
        config.observe.backend = "redis".into();
        config.redis.url = "not-a-valid-url".into();
        // Should fall back to log sink instead of panicking.
        let _sink = create_event_sink(&config);
    }
}
