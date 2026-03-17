use async_trait::async_trait;
use opentelemetry::KeyValue;
use opentelemetry::trace::{Span, SpanKind, Status, Tracer};
use orka_core::traits::EventSink;
use orka_core::{DomainEvent, DomainEventKind};

/// Event sink that emits domain events as OpenTelemetry spans.
pub struct OtelEventSink {
    tracer: opentelemetry_sdk::trace::Tracer,
}

impl OtelEventSink {
    pub fn new(tracer: opentelemetry_sdk::trace::Tracer) -> Self {
        Self { tracer }
    }
}

#[async_trait]
impl EventSink for OtelEventSink {
    async fn emit(&self, event: DomainEvent) {
        let (span_name, attributes) = match &event.kind {
            DomainEventKind::MessageReceived {
                message_id,
                channel,
                session_id,
            } => (
                "message.received",
                vec![
                    KeyValue::new("message_id", message_id.to_string()),
                    KeyValue::new("channel", channel.clone()),
                    KeyValue::new("session_id", session_id.to_string()),
                ],
            ),
            DomainEventKind::SessionCreated {
                session_id,
                channel,
            } => (
                "session.created",
                vec![
                    KeyValue::new("session_id", session_id.to_string()),
                    KeyValue::new("channel", channel.clone()),
                ],
            ),
            DomainEventKind::HandlerInvoked {
                message_id,
                session_id,
            } => (
                "handler.invoked",
                vec![
                    KeyValue::new("message_id", message_id.to_string()),
                    KeyValue::new("session_id", session_id.to_string()),
                ],
            ),
            DomainEventKind::HandlerCompleted {
                message_id,
                session_id,
                duration_ms,
                reply_count,
            } => (
                "handler.completed",
                vec![
                    KeyValue::new("message_id", message_id.to_string()),
                    KeyValue::new("session_id", session_id.to_string()),
                    KeyValue::new("duration_ms", *duration_ms as i64),
                    KeyValue::new("reply_count", *reply_count as i64),
                ],
            ),
            DomainEventKind::SkillInvoked {
                skill_name,
                message_id,
            } => (
                "skill.invoked",
                vec![
                    KeyValue::new("skill_name", skill_name.clone()),
                    KeyValue::new("message_id", message_id.to_string()),
                ],
            ),
            DomainEventKind::SkillCompleted {
                skill_name,
                message_id,
                duration_ms,
                success,
            } => (
                "skill.completed",
                vec![
                    KeyValue::new("skill_name", skill_name.clone()),
                    KeyValue::new("message_id", message_id.to_string()),
                    KeyValue::new("duration_ms", *duration_ms as i64),
                    KeyValue::new("success", *success),
                ],
            ),
            DomainEventKind::LlmCompleted {
                message_id,
                model,
                input_tokens,
                output_tokens,
                duration_ms,
                estimated_cost_usd,
            } => {
                let mut attrs = vec![
                    KeyValue::new("message_id", message_id.to_string()),
                    KeyValue::new("model", model.clone()),
                    KeyValue::new("input_tokens", *input_tokens as i64),
                    KeyValue::new("output_tokens", *output_tokens as i64),
                    KeyValue::new("duration_ms", *duration_ms as i64),
                ];
                if let Some(cost) = estimated_cost_usd {
                    attrs.push(KeyValue::new("estimated_cost_usd", *cost));
                }
                ("llm.completed", attrs)
            }
            DomainEventKind::ErrorOccurred { source, message } => (
                "error.occurred",
                vec![
                    KeyValue::new("error.source", source.clone()),
                    KeyValue::new("error.message", message.clone()),
                ],
            ),
            DomainEventKind::AgentReasoning {
                message_id,
                iteration,
                reasoning_text,
            } => (
                "agent.reasoning",
                vec![
                    KeyValue::new("message_id", message_id.to_string()),
                    KeyValue::new("iteration", *iteration as i64),
                    KeyValue::new("reasoning_len", reasoning_text.len() as i64),
                ],
            ),
            DomainEventKind::AgentIteration {
                message_id,
                iteration,
                tool_count,
                tokens_used,
                elapsed_ms,
            } => (
                "agent.iteration",
                vec![
                    KeyValue::new("message_id", message_id.to_string()),
                    KeyValue::new("iteration", *iteration as i64),
                    KeyValue::new("tool_count", *tool_count as i64),
                    KeyValue::new("tokens_used", *tokens_used as i64),
                    KeyValue::new("elapsed_ms", *elapsed_ms as i64),
                ],
            ),
            DomainEventKind::PrivilegedCommandExecuted {
                message_id,
                session_id,
                command,
                success,
                duration_ms,
                ..
            } => (
                "privileged_command.executed",
                vec![
                    KeyValue::new("message_id", message_id.to_string()),
                    KeyValue::new("session_id", session_id.to_string()),
                    KeyValue::new("command", command.clone()),
                    KeyValue::new("success", *success),
                    KeyValue::new("duration_ms", *duration_ms as i64),
                ],
            ),
            DomainEventKind::PrivilegedCommandDenied {
                message_id,
                session_id,
                command,
                reason,
                ..
            } => (
                "privileged_command.denied",
                vec![
                    KeyValue::new("message_id", message_id.to_string()),
                    KeyValue::new("session_id", session_id.to_string()),
                    KeyValue::new("command", command.clone()),
                    KeyValue::new("reason", reason.clone()),
                ],
            ),
            DomainEventKind::PrinciplesInjected { session_id, count } => (
                "experience.principles_injected",
                vec![
                    KeyValue::new("session_id", session_id.to_string()),
                    KeyValue::new("count", *count as i64),
                ],
            ),
            DomainEventKind::ReflectionCompleted {
                session_id,
                principles_created,
                trajectory_id,
            } => (
                "experience.reflection_completed",
                vec![
                    KeyValue::new("session_id", session_id.to_string()),
                    KeyValue::new("principles_created", *principles_created as i64),
                    KeyValue::new("trajectory_id", trajectory_id.clone()),
                ],
            ),
            DomainEventKind::Heartbeat => ("heartbeat", vec![]),
            _ => ("unknown", vec![]),
        };

        let mut span = self
            .tracer
            .span_builder(span_name.to_string())
            .with_kind(SpanKind::Internal)
            .with_attributes(attributes)
            .start(&self.tracer);

        if matches!(&event.kind, DomainEventKind::ErrorOccurred { .. }) {
            span.set_status(Status::error(""));
        }

        span.end();
    }
}

/// Initialize OpenTelemetry with OTLP exporter.
/// Returns a tracer that can be used to create the OtelEventSink.
pub fn init_otel_tracer(
    service_name: &str,
) -> Result<opentelemetry_sdk::trace::Tracer, Box<dyn std::error::Error>> {
    use opentelemetry::trace::TracerProvider;
    use opentelemetry_otlp::SpanExporter;
    use opentelemetry_sdk::trace::SdkTracerProvider;

    let exporter = SpanExporter::builder().with_tonic().build()?;

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .build();

    let tracer = provider.tracer(service_name.to_string());

    // Set as global provider
    opentelemetry::global::set_tracer_provider(provider);

    Ok(tracer)
}
