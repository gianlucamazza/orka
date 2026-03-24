use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use opentelemetry::{
    KeyValue,
    trace::{Span, SpanKind, Status, Tracer},
};
use orka_core::{DomainEvent, DomainEventKind, traits::EventSink};

/// Event sink that emits domain events as OpenTelemetry spans following the
/// [OTel GenAI semantic conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/).
pub struct OtelEventSink {
    tracer: opentelemetry_sdk::trace::Tracer,
}

impl OtelEventSink {
    /// Create a new [`OtelEventSink`] using the given tracer.
    pub fn new(tracer: opentelemetry_sdk::trace::Tracer) -> Self {
        Self { tracer }
    }
}

#[async_trait]
impl EventSink for OtelEventSink {
    async fn emit(&self, event: DomainEvent) {
        match &event.kind {
            // ── Tool execution (GenAI: execute_tool, CLIENT) ─────────────────────────
            // SkillInvoked is a no-op at the OTel level: the full span is emitted
            // at SkillCompleted where we have accurate timing.
            DomainEventKind::SkillInvoked { .. } => {}

            DomainEventKind::SkillCompleted {
                skill_name,
                message_id,
                duration_ms,
                success,
                error_category,
                error_message,
                ..
            } => {
                let start_time = SystemTime::now()
                    .checked_sub(Duration::from_millis(*duration_ms))
                    .unwrap_or(SystemTime::now());

                let mut attrs = vec![
                    KeyValue::new("gen_ai.tool.name", skill_name.clone()),
                    KeyValue::new("gen_ai.tool.call.id", message_id.to_string()),
                    KeyValue::new("duration_ms", *duration_ms as i64),
                ];
                if let Some(cat) = error_category {
                    attrs.push(KeyValue::new("error.type", format!("{cat:?}")));
                }

                let mut span = self
                    .tracer
                    .span_builder(format!("execute_tool {skill_name}"))
                    .with_kind(SpanKind::Client)
                    .with_start_time(start_time)
                    .with_attributes(attrs)
                    .start(&self.tracer);

                if !success {
                    let msg = error_message.as_deref().unwrap_or("skill execution failed");
                    span.set_status(Status::error(msg.to_string()));
                }

                span.end();
            }

            // ── LLM request (pre-call marker) ────────────────────────────────────────
            DomainEventKind::LlmRequest {
                message_id,
                model,
                provider,
                iteration,
            } => {
                let attrs = vec![
                    KeyValue::new("gen_ai.system", provider.clone()),
                    KeyValue::new("gen_ai.request.model", model.clone()),
                    KeyValue::new("gen_ai.tool.call.id", message_id.to_string()),
                    KeyValue::new("gen_ai.agent.iteration", *iteration as i64),
                ];

                let mut span = self
                    .tracer
                    .span_builder(format!("gen_ai.request {model}"))
                    .with_kind(SpanKind::Client)
                    .with_attributes(attrs)
                    .start(&self.tracer);

                span.end();
            }

            // ── LLM completion (GenAI: gen_ai.request, CLIENT) ───────────────────────
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
                let start_time = SystemTime::now()
                    .checked_sub(Duration::from_millis(*duration_ms))
                    .unwrap_or(SystemTime::now());

                let mut attrs = vec![
                    KeyValue::new("gen_ai.system", provider.clone()),
                    KeyValue::new("gen_ai.request.model", model.clone()),
                    KeyValue::new("gen_ai.usage.input_tokens", *input_tokens as i64),
                    KeyValue::new("gen_ai.usage.output_tokens", *output_tokens as i64),
                    KeyValue::new("gen_ai.usage.reasoning_tokens", *reasoning_tokens as i64),
                    KeyValue::new("gen_ai.tool.call.id", message_id.to_string()),
                    KeyValue::new("duration_ms", *duration_ms as i64),
                ];
                if let Some(cost) = estimated_cost_usd {
                    attrs.push(KeyValue::new("gen_ai.usage.cost_usd", *cost));
                }

                let mut span = self
                    .tracer
                    .span_builder(format!("gen_ai.request {model}"))
                    .with_kind(SpanKind::Client)
                    .with_start_time(start_time)
                    .with_attributes(attrs)
                    .start(&self.tracer);

                span.end();
            }

            // ── Agent iteration (GenAI: invoke_agent, SERVER) ────────────────────────
            DomainEventKind::AgentIteration {
                message_id,
                iteration,
                tool_count,
                tokens_used,
                elapsed_ms,
            } => {
                let start_time = SystemTime::now()
                    .checked_sub(Duration::from_millis(*elapsed_ms))
                    .unwrap_or(SystemTime::now());

                let attrs = vec![
                    KeyValue::new("gen_ai.tool.call.id", message_id.to_string()),
                    KeyValue::new("gen_ai.agent.iteration", *iteration as i64),
                    KeyValue::new("gen_ai.tool.calls.count", *tool_count as i64),
                    KeyValue::new("gen_ai.usage.tokens", *tokens_used as i64),
                    KeyValue::new("elapsed_ms", *elapsed_ms as i64),
                ];

                let mut span = self
                    .tracer
                    .span_builder("invoke_agent")
                    .with_kind(SpanKind::Server)
                    .with_start_time(start_time)
                    .with_attributes(attrs)
                    .start(&self.tracer);

                span.end();
            }

            // ── Errors ───────────────────────────────────────────────────────────────
            DomainEventKind::ErrorOccurred { source, message } => {
                let attrs = vec![
                    KeyValue::new("error.type", source.clone()),
                    KeyValue::new("error.message", message.clone()),
                ];

                let mut span = self
                    .tracer
                    .span_builder("error")
                    .with_kind(SpanKind::Internal)
                    .with_attributes(attrs)
                    .start(&self.tracer);

                span.set_status(Status::error(message.clone()));
                span.end();
            }

            // ── Other events (not GenAI-specific, keep as internal spans) ────────────
            DomainEventKind::MessageReceived {
                message_id,
                channel,
                session_id,
            } => {
                let attrs = vec![
                    KeyValue::new("message_id", message_id.to_string()),
                    KeyValue::new("channel", channel.clone()),
                    KeyValue::new("session_id", session_id.to_string()),
                ];
                self.emit_internal("message.received", attrs, false).await;
            }
            DomainEventKind::SessionCreated {
                session_id,
                channel,
            } => {
                let attrs = vec![
                    KeyValue::new("session_id", session_id.to_string()),
                    KeyValue::new("channel", channel.clone()),
                ];
                self.emit_internal("session.created", attrs, false).await;
            }
            DomainEventKind::HandlerInvoked {
                message_id,
                session_id,
            } => {
                let attrs = vec![
                    KeyValue::new("message_id", message_id.to_string()),
                    KeyValue::new("session_id", session_id.to_string()),
                ];
                self.emit_internal("handler.invoked", attrs, false).await;
            }
            DomainEventKind::HandlerCompleted {
                message_id,
                session_id,
                duration_ms,
                reply_count,
            } => {
                let attrs = vec![
                    KeyValue::new("message_id", message_id.to_string()),
                    KeyValue::new("session_id", session_id.to_string()),
                    KeyValue::new("duration_ms", *duration_ms as i64),
                    KeyValue::new("reply_count", *reply_count as i64),
                ];
                self.emit_internal("handler.completed", attrs, false).await;
            }
            DomainEventKind::AgentReasoning {
                message_id,
                iteration,
                reasoning_text,
            } => {
                let attrs = vec![
                    KeyValue::new("message_id", message_id.to_string()),
                    KeyValue::new("gen_ai.agent.iteration", *iteration as i64),
                    KeyValue::new("reasoning_len", reasoning_text.len() as i64),
                ];
                self.emit_internal("agent.reasoning", attrs, false).await;
            }
            DomainEventKind::PrivilegedCommandExecuted {
                message_id,
                session_id,
                command,
                success,
                duration_ms,
                ..
            } => {
                let attrs = vec![
                    KeyValue::new("message_id", message_id.to_string()),
                    KeyValue::new("session_id", session_id.to_string()),
                    KeyValue::new("command", command.clone()),
                    KeyValue::new("success", *success),
                    KeyValue::new("duration_ms", *duration_ms as i64),
                ];
                self.emit_internal("privileged_command.executed", attrs, false)
                    .await;
            }
            DomainEventKind::PrivilegedCommandDenied {
                message_id,
                session_id,
                command,
                reason,
                ..
            } => {
                let attrs = vec![
                    KeyValue::new("message_id", message_id.to_string()),
                    KeyValue::new("session_id", session_id.to_string()),
                    KeyValue::new("command", command.clone()),
                    KeyValue::new("reason", reason.clone()),
                ];
                self.emit_internal("privileged_command.denied", attrs, false)
                    .await;
            }
            DomainEventKind::PrinciplesInjected { session_id, count } => {
                let attrs = vec![
                    KeyValue::new("session_id", session_id.to_string()),
                    KeyValue::new("count", *count as i64),
                ];
                self.emit_internal("experience.principles_injected", attrs, false)
                    .await;
            }
            DomainEventKind::ReflectionCompleted {
                session_id,
                principles_created,
                trajectory_id,
            } => {
                let attrs = vec![
                    KeyValue::new("session_id", session_id.to_string()),
                    KeyValue::new("principles_created", *principles_created as i64),
                    KeyValue::new("trajectory_id", trajectory_id.clone()),
                ];
                self.emit_internal("experience.reflection_completed", attrs, false)
                    .await;
            }
            DomainEventKind::TrajectoryRecorded {
                session_id,
                trajectory_id,
            } => {
                let attrs = vec![
                    KeyValue::new("session_id", session_id.to_string()),
                    KeyValue::new("trajectory_id", trajectory_id.clone()),
                ];
                self.emit_internal("experience.trajectory_recorded", attrs, false)
                    .await;
            }
            DomainEventKind::DistillationCompleted {
                workspace,
                principles_created,
            } => {
                let attrs = vec![
                    KeyValue::new("workspace", workspace.clone()),
                    KeyValue::new("principles_created", *principles_created as i64),
                ];
                self.emit_internal("experience.distillation_completed", attrs, false)
                    .await;
            }
            DomainEventKind::SkillDisabled {
                skill_name,
                reason,
                source,
            } => {
                let attrs = vec![
                    KeyValue::new("gen_ai.tool.name", skill_name.clone()),
                    KeyValue::new("reason", reason.clone()),
                    KeyValue::new("source", source.clone()),
                ];
                self.emit_internal("skill.disabled", attrs, false).await;
            }
            DomainEventKind::Heartbeat => {}
            _ => {}
        }
    }
}

impl OtelEventSink {
    /// Emit a generic internal span with the given name and attributes.
    async fn emit_internal(&self, name: &str, attrs: Vec<KeyValue>, is_error: bool) {
        let mut span = self
            .tracer
            .span_builder(name.to_string())
            .with_kind(SpanKind::Internal)
            .with_attributes(attrs)
            .start(&self.tracer);

        if is_error {
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
