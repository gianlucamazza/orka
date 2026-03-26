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
#[allow(clippy::too_many_lines)]
impl EventSink for OtelEventSink {
    async fn emit(&self, event: DomainEvent) {
        match &event.kind {
            // ── Tool execution (GenAI: execute_tool, CLIENT) ─────────────────────────
            // SkillInvoked is a no-op at the OTel level: the full span is emitted
            // at SkillCompleted where we have accurate timing.
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
                    KeyValue::new("gen_ai.usage.input_tokens", i64::from(*input_tokens)),
                    KeyValue::new("gen_ai.usage.output_tokens", i64::from(*output_tokens)),
                    KeyValue::new("gen_ai.usage.reasoning_tokens", i64::from(*reasoning_tokens)),
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
                self.emit_internal("message.received", attrs, false);
            }
            DomainEventKind::SessionCreated {
                session_id,
                channel,
            } => {
                let attrs = vec![
                    KeyValue::new("session_id", session_id.to_string()),
                    KeyValue::new("channel", channel.clone()),
                ];
                self.emit_internal("session.created", attrs, false);
            }
            DomainEventKind::HandlerInvoked {
                message_id,
                session_id,
            } => {
                let attrs = vec![
                    KeyValue::new("message_id", message_id.to_string()),
                    KeyValue::new("session_id", session_id.to_string()),
                ];
                self.emit_internal("handler.invoked", attrs, false);
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
                self.emit_internal("handler.completed", attrs, false);
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
                self.emit_internal("agent.reasoning", attrs, false);
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
                self.emit_internal("privileged_command.executed", attrs, false);
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
                self.emit_internal("privileged_command.denied", attrs, false);
            }
            DomainEventKind::PrinciplesInjected { session_id, count } => {
                let attrs = vec![
                    KeyValue::new("session_id", session_id.to_string()),
                    KeyValue::new("count", *count as i64),
                ];
                self.emit_internal("experience.principles_injected", attrs, false);
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
                self.emit_internal("experience.reflection_completed", attrs, false);
            }
            DomainEventKind::TrajectoryRecorded {
                session_id,
                trajectory_id,
            } => {
                let attrs = vec![
                    KeyValue::new("session_id", session_id.to_string()),
                    KeyValue::new("trajectory_id", trajectory_id.clone()),
                ];
                self.emit_internal("experience.trajectory_recorded", attrs, false);
            }
            DomainEventKind::DistillationCompleted {
                workspace,
                principles_created,
            } => {
                let attrs = vec![
                    KeyValue::new("workspace", workspace.clone()),
                    KeyValue::new("principles_created", *principles_created as i64),
                ];
                self.emit_internal("experience.distillation_completed", attrs, false);
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
                self.emit_internal("skill.disabled", attrs, false);
            }
            _ => {}
        }
    }
}

impl OtelEventSink {
    /// Emit a generic internal span with the given name and attributes.
    fn emit_internal(&self, name: &str, attrs: Vec<KeyValue>, is_error: bool) {
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
/// Returns a tracer that can be used to create the `OtelEventSink`.
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use opentelemetry::trace::{SpanKind, TracerProvider as _};
    use opentelemetry_sdk::{
        error::OTelSdkResult,
        trace::{SdkTracerProvider, SpanData, SpanExporter},
    };
    use orka_core::traits::EventSink;

    use super::*;
    use crate::test_helpers::{all_event_kinds, make_event};

    /// Collecting exporter for use in tests — gathers exported spans into a
    /// Vec.
    #[derive(Debug, Clone)]
    struct CollectingExporter(Arc<Mutex<Vec<SpanData>>>);

    impl CollectingExporter {
        fn new() -> (Self, Arc<Mutex<Vec<SpanData>>>) {
            let store = Arc::new(Mutex::new(Vec::new()));
            (Self(store.clone()), store)
        }
    }

    impl SpanExporter for CollectingExporter {
        fn export(
            &mut self,
            batch: Vec<SpanData>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = OTelSdkResult> + Send + 'static>>
        {
            self.0.lock().unwrap().extend(batch);
            Box::pin(std::future::ready(Ok(())))
        }

        fn shutdown(&mut self) -> OTelSdkResult {
            Ok(())
        }

        fn force_flush(&mut self) -> OTelSdkResult {
            Ok(())
        }
    }

    fn make_sink() -> (OtelEventSink, Arc<Mutex<Vec<SpanData>>>) {
        let (exporter, store) = CollectingExporter::new();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter)
            .build();
        let tracer = provider.tracer("test");
        (OtelEventSink::new(tracer), store)
    }

    fn span_names(store: &Arc<Mutex<Vec<SpanData>>>) -> Vec<String> {
        store
            .lock()
            .unwrap()
            .iter()
            .map(|s| s.name.to_string())
            .collect()
    }

    #[tokio::test]
    async fn all_event_variants_emit_without_panic() {
        let (sink, _) = make_sink();
        for kind in all_event_kinds() {
            sink.emit(make_event(kind)).await;
        }
    }

    #[tokio::test]
    async fn skill_completed_creates_client_span() {
        let (sink, store) = make_sink();
        sink.emit(make_event(orka_core::DomainEventKind::SkillCompleted {
            skill_name: "web_search".into(),
            message_id: orka_core::types::MessageId::new(),
            duration_ms: 100,
            success: true,
            error_category: None,
            output_preview: None,
            error_message: None,
        }))
        .await;

        let spans = store.lock().unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].name.as_ref(), "execute_tool web_search");
        assert_eq!(spans[0].span_kind, SpanKind::Client);
    }

    #[tokio::test]
    async fn skill_completed_with_error_sets_error_status() {
        let (sink, store) = make_sink();
        sink.emit(make_event(orka_core::DomainEventKind::SkillCompleted {
            skill_name: "web_search".into(),
            message_id: orka_core::types::MessageId::new(),
            duration_ms: 50,
            success: false,
            error_category: None,
            output_preview: None,
            error_message: Some("timeout".into()),
        }))
        .await;

        let spans = store.lock().unwrap();
        assert!(
            matches!(spans[0].status, opentelemetry::trace::Status::Error { .. }),
            "expected Error status for failed skill"
        );
    }

    #[tokio::test]
    async fn llm_completed_creates_client_span_with_model_attribute() {
        let (sink, store) = make_sink();
        sink.emit(make_event(orka_core::DomainEventKind::LlmCompleted {
            message_id: orka_core::types::MessageId::new(),
            model: "claude-sonnet-4-6".into(),
            provider: "anthropic".into(),
            input_tokens: 100,
            output_tokens: 50,
            reasoning_tokens: 0,
            duration_ms: 300,
            estimated_cost_usd: Some(0.001),
        }))
        .await;

        let spans = store.lock().unwrap();
        assert_eq!(spans[0].span_kind, SpanKind::Client);
        let has_model_attr = spans[0]
            .attributes
            .iter()
            .any(|kv| kv.key.as_str() == "gen_ai.request.model");
        assert!(has_model_attr, "expected gen_ai.request.model attribute");
    }

    #[tokio::test]
    async fn agent_iteration_creates_server_span() {
        let (sink, store) = make_sink();
        sink.emit(make_event(orka_core::DomainEventKind::AgentIteration {
            message_id: orka_core::types::MessageId::new(),
            iteration: 3,
            tool_count: 2,
            tokens_used: 500,
            elapsed_ms: 1000,
        }))
        .await;

        let spans = store.lock().unwrap();
        assert_eq!(spans[0].name.as_ref(), "invoke_agent");
        assert_eq!(spans[0].span_kind, SpanKind::Server);
    }

    #[tokio::test]
    async fn message_received_creates_internal_span() {
        let (sink, store) = make_sink();
        sink.emit(make_event(orka_core::DomainEventKind::MessageReceived {
            message_id: orka_core::types::MessageId::new(),
            channel: "telegram".into(),
            session_id: orka_core::types::SessionId::new(),
        }))
        .await;

        let names = span_names(&store);
        assert!(
            names.iter().any(|n| n == "message.received"),
            "expected message.received span, got: {names:?}"
        );
        let spans = store.lock().unwrap();
        assert_eq!(spans[0].span_kind, SpanKind::Internal);
    }

    #[tokio::test]
    async fn error_occurred_creates_error_span() {
        let (sink, store) = make_sink();
        sink.emit(make_event(orka_core::DomainEventKind::ErrorOccurred {
            source: "adapter".into(),
            message: "connection lost".into(),
        }))
        .await;

        let spans = store.lock().unwrap();
        assert_eq!(spans[0].name.as_ref(), "error");
        assert!(matches!(
            spans[0].status,
            opentelemetry::trace::Status::Error { .. }
        ));
    }
}
