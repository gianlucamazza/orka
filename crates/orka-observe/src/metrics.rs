use metrics::{counter, histogram};
use metrics_exporter_prometheus::PrometheusBuilder;
use orka_core::{DomainEvent, DomainEventKind};

/// Install the Prometheus metrics recorder. Returns the render handle for `/metrics`.
///
/// Call this once at startup. Returns `None` if already installed.
pub fn install_prometheus_recorder() -> Option<PrometheusHandle> {
    let builder = PrometheusBuilder::new();
    match builder.install_recorder() {
        Ok(handle) => Some(PrometheusHandle(handle)),
        Err(_) => None,
    }
}

/// Handle to render Prometheus metrics.
pub struct PrometheusHandle(metrics_exporter_prometheus::PrometheusHandle);

impl PrometheusHandle {
    /// Render current metrics in Prometheus text exposition format.
    pub fn render(&self) -> String {
        self.0.render()
    }
}

/// Record metrics from a domain event.
pub fn record_event(event: &DomainEvent) {
    match &event.kind {
        DomainEventKind::MessageReceived { channel, .. } => {
            counter!("orka_messages_received_total", "channel" => channel.clone()).increment(1);
        }
        DomainEventKind::SessionCreated { channel, .. } => {
            counter!("orka_sessions_created_total", "channel" => channel.clone()).increment(1);
        }
        DomainEventKind::HandlerInvoked { .. } => {
            counter!("orka_handler_invocations_total").increment(1);
        }
        DomainEventKind::HandlerCompleted { duration_ms, .. } => {
            counter!("orka_handler_completions_total").increment(1);
            histogram!("orka_handler_duration_seconds").record(*duration_ms as f64 / 1000.0);
        }
        DomainEventKind::SkillInvoked { skill_name, .. } => {
            counter!("orka_skill_invocations_total", "skill" => skill_name.clone()).increment(1);
        }
        DomainEventKind::SkillCompleted {
            skill_name,
            duration_ms,
            success,
            ..
        } => {
            let status = if *success { "ok" } else { "error" };
            counter!("orka_skill_completions_total", "skill" => skill_name.clone(), "status" => status).increment(1);
            histogram!("orka_skill_duration_seconds", "skill" => skill_name.clone())
                .record(*duration_ms as f64 / 1000.0);
        }
        DomainEventKind::LlmCompleted {
            model,
            input_tokens,
            output_tokens,
            duration_ms,
            ..
        } => {
            counter!("orka_llm_completions_total", "model" => model.clone()).increment(1);
            counter!("orka_llm_input_tokens_total", "model" => model.clone())
                .increment(*input_tokens as u64);
            counter!("orka_llm_output_tokens_total", "model" => model.clone())
                .increment(*output_tokens as u64);
            histogram!("orka_llm_duration_seconds", "model" => model.clone())
                .record(*duration_ms as f64 / 1000.0);
        }
        DomainEventKind::ErrorOccurred { source, .. } => {
            counter!("orka_errors_total", "source" => source.clone()).increment(1);
        }
        DomainEventKind::Heartbeat => {
            counter!("orka_heartbeats_total").increment(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orka_core::types::{EventId, MessageId, SessionId};
    use std::collections::HashMap;

    fn make_event(kind: DomainEventKind) -> DomainEvent {
        DomainEvent {
            id: EventId::new(),
            timestamp: chrono::Utc::now(),
            kind,
            metadata: HashMap::new(),
        }
    }

    fn all_event_kinds() -> Vec<DomainEventKind> {
        let mid = MessageId::new();
        let sid = SessionId::new();
        vec![
            DomainEventKind::MessageReceived {
                message_id: mid.clone(),
                channel: "telegram".into(),
                session_id: sid.clone(),
            },
            DomainEventKind::SessionCreated {
                session_id: sid.clone(),
                channel: "discord".into(),
            },
            DomainEventKind::HandlerInvoked {
                message_id: mid.clone(),
                session_id: sid.clone(),
            },
            DomainEventKind::HandlerCompleted {
                message_id: mid.clone(),
                session_id: sid.clone(),
                duration_ms: 150,
                reply_count: 2,
            },
            DomainEventKind::SkillInvoked {
                skill_name: "weather".into(),
                message_id: mid.clone(),
            },
            DomainEventKind::SkillCompleted {
                skill_name: "weather".into(),
                message_id: mid.clone(),
                duration_ms: 80,
                success: true,
            },
            DomainEventKind::SkillCompleted {
                skill_name: "weather".into(),
                message_id: mid.clone(),
                duration_ms: 30,
                success: false,
            },
            DomainEventKind::LlmCompleted {
                message_id: mid.clone(),
                model: "gpt-4".into(),
                input_tokens: 500,
                output_tokens: 200,
                duration_ms: 1200,
            },
            DomainEventKind::ErrorOccurred {
                source: "adapter".into(),
                message: "connection lost".into(),
            },
            DomainEventKind::Heartbeat,
        ]
    }

    #[test]
    fn record_event_does_not_panic_for_all_variants() {
        // The metrics crate uses a NoopRecorder by default when no recorder
        // is installed, so all counter/histogram calls are safe no-ops.
        for kind in all_event_kinds() {
            record_event(&make_event(kind));
        }
    }

    #[test]
    fn record_event_handles_zero_duration_and_tokens() {
        let mid = MessageId::new();
        let sid = SessionId::new();

        let edge_cases = vec![
            DomainEventKind::HandlerCompleted {
                message_id: mid.clone(),
                session_id: sid.clone(),
                duration_ms: 0,
                reply_count: 0,
            },
            DomainEventKind::SkillCompleted {
                skill_name: "test".into(),
                message_id: mid.clone(),
                duration_ms: 0,
                success: true,
            },
            DomainEventKind::LlmCompleted {
                message_id: mid.clone(),
                model: "m".into(),
                input_tokens: 0,
                output_tokens: 0,
                duration_ms: 0,
            },
        ];

        for kind in edge_cases {
            record_event(&make_event(kind));
        }
    }
}
