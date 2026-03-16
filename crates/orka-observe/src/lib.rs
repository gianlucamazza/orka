use std::sync::Arc;

use async_trait::async_trait;
use orka_core::config::OrkaConfig;
use orka_core::traits::EventSink;
use orka_core::{DomainEvent, DomainEventKind};
use tracing::{info, warn};

pub mod metrics;
pub mod otel_sink;
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
            } => {
                info!(%message_id, model, input_tokens, output_tokens, duration_ms, "llm completed");
            }
            DomainEventKind::ErrorOccurred { source, message } => {
                warn!(source, message, "error occurred");
            }
            DomainEventKind::Heartbeat => {
                info!("heartbeat");
            }
        }
    }
}

pub fn create_event_sink(config: &OrkaConfig) -> Arc<dyn EventSink> {
    match config.observe.backend.as_str() {
        "redis" => match redis_sink::RedisEventSink::new(&config.redis.url, config.observe.batch_size, config.observe.flush_interval_ms) {
            Ok(sink) => {
                info!("event sink: Redis Streams");
                Arc::new(sink)
            }
            Err(e) => {
                warn!(%e, "failed to create Redis event sink, falling back to log");
                Arc::new(LogEventSink)
            }
        },
        "otel" | "otlp" => {
            match otel_sink::init_otel_tracer("orka") {
                Ok(tracer) => {
                    info!("event sink: OpenTelemetry (OTLP)");
                    Arc::new(otel_sink::OtelEventSink::new(tracer))
                }
                Err(e) => {
                    warn!(%e, "failed to initialize OTel, falling back to log");
                    Arc::new(LogEventSink)
                }
            }
        },
        _ => {
            info!("event sink: log");
            Arc::new(LogEventSink)
        }
    }
}
