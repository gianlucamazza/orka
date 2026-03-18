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
            estimated_cost_usd,
            ..
        } => {
            counter!("orka_llm_completions_total", "model" => model.clone()).increment(1);
            counter!("orka_llm_input_tokens_total", "model" => model.clone())
                .increment(*input_tokens as u64);
            counter!("orka_llm_output_tokens_total", "model" => model.clone())
                .increment(*output_tokens as u64);
            histogram!("orka_llm_duration_seconds", "model" => model.clone())
                .record(*duration_ms as f64 / 1000.0);
            if let Some(cost) = estimated_cost_usd {
                counter!("orka_llm_cost_dollars_total", "model" => model.clone())
                    .increment((*cost * 1_000_000.0) as u64);
            }
        }
        DomainEventKind::ErrorOccurred { source, .. } => {
            counter!("orka_errors_total", "source" => source.clone()).increment(1);
        }
        DomainEventKind::AgentReasoning { .. } => {
            counter!("orka_agent_reasoning_total").increment(1);
        }
        DomainEventKind::AgentIteration {
            tool_count,
            tokens_used,
            elapsed_ms,
            ..
        } => {
            counter!("orka_agent_iterations_total").increment(1);
            counter!("orka_agent_iteration_tools_total").increment(*tool_count as u64);
            counter!("orka_agent_iteration_tokens_total").increment(*tokens_used);
            histogram!("orka_agent_iteration_duration_seconds").record(*elapsed_ms as f64 / 1000.0);
        }
        DomainEventKind::PrivilegedCommandExecuted {
            command,
            success,
            duration_ms,
            ..
        } => {
            let status = if *success { "ok" } else { "error" };
            counter!("orka_privileged_commands_total", "command" => command.clone(), "status" => status).increment(1);
            histogram!("orka_privileged_command_duration_seconds", "command" => command.clone())
                .record(*duration_ms as f64 / 1000.0);
        }
        DomainEventKind::PrivilegedCommandDenied { command, .. } => {
            counter!("orka_privileged_commands_denied_total", "command" => command.clone())
                .increment(1);
        }
        DomainEventKind::PrinciplesInjected { count, .. } => {
            counter!("orka_principles_injected_total").increment(*count as u64);
        }
        DomainEventKind::ReflectionCompleted {
            principles_created, ..
        } => {
            counter!("orka_reflections_completed_total").increment(1);
            counter!("orka_principles_created_total").increment(*principles_created as u64);
        }
        DomainEventKind::TrajectoryRecorded { .. } => {
            counter!("orka_trajectories_recorded_total").increment(1);
        }
        DomainEventKind::DistillationCompleted {
            principles_created, ..
        } => {
            counter!("orka_distillations_completed_total").increment(1);
            counter!("orka_principles_created_total").increment(*principles_created as u64);
        }
        DomainEventKind::Heartbeat => {
            counter!("orka_heartbeats_total").increment(1);
        }
        other => {
            tracing::trace!(?other, "no metric registered for domain event kind");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orka_core::types::{MessageId, SessionId};

    fn make_event(kind: DomainEventKind) -> DomainEvent {
        DomainEvent::new(kind)
    }

    fn all_event_kinds() -> Vec<DomainEventKind> {
        let mid = MessageId::new();
        let sid = SessionId::new();
        vec![
            DomainEventKind::MessageReceived {
                message_id: mid,
                channel: "telegram".into(),
                session_id: sid,
            },
            DomainEventKind::SessionCreated {
                session_id: sid,
                channel: "discord".into(),
            },
            DomainEventKind::HandlerInvoked {
                message_id: mid,
                session_id: sid,
            },
            DomainEventKind::HandlerCompleted {
                message_id: mid,
                session_id: sid,
                duration_ms: 150,
                reply_count: 2,
            },
            DomainEventKind::SkillInvoked {
                skill_name: "weather".into(),
                message_id: mid,
            },
            DomainEventKind::SkillCompleted {
                skill_name: "weather".into(),
                message_id: mid,
                duration_ms: 80,
                success: true,
            },
            DomainEventKind::SkillCompleted {
                skill_name: "weather".into(),
                message_id: mid,
                duration_ms: 30,
                success: false,
            },
            DomainEventKind::LlmCompleted {
                message_id: mid,
                model: "gpt-4".into(),
                input_tokens: 500,
                output_tokens: 200,
                duration_ms: 1200,
                estimated_cost_usd: Some(0.01),
            },
            DomainEventKind::ErrorOccurred {
                source: "adapter".into(),
                message: "connection lost".into(),
            },
            DomainEventKind::AgentReasoning {
                message_id: mid,
                iteration: 1,
                reasoning_text: "Thinking...".into(),
            },
            DomainEventKind::AgentIteration {
                message_id: mid,
                iteration: 1,
                tool_count: 3,
                tokens_used: 1000,
                elapsed_ms: 2000,
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
                duration_ms: 100,
            },
            DomainEventKind::PrivilegedCommandDenied {
                message_id: mid,
                session_id: sid,
                command: "rm".into(),
                args: vec!["-rf".into()],
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
                message_id: mid,
                session_id: sid,
                duration_ms: 0,
                reply_count: 0,
            },
            DomainEventKind::SkillCompleted {
                skill_name: "test".into(),
                message_id: mid,
                duration_ms: 0,
                success: true,
            },
            DomainEventKind::LlmCompleted {
                message_id: mid,
                model: "m".into(),
                input_tokens: 0,
                output_tokens: 0,
                duration_ms: 0,
                estimated_cost_usd: None,
            },
        ];

        for kind in edge_cases {
            record_event(&make_event(kind));
        }
    }
}
