//! Shared domain-event helpers for privileged OS skills.
//!
//! `shell_exec`, `package_install`, and `service_control` all emit the same
//! two event kinds when a sudo-gated command runs.  This module provides the
//! canonical implementations to avoid copy-paste across skill files.

use orka_core::{DomainEvent, DomainEventKind, SkillInput};

/// Emit a [`DomainEventKind::PrivilegedCommandExecuted`] event if an event
/// sink is attached to the skill context.
pub async fn emit_executed(
    input: &SkillInput,
    command: &str,
    args: &[&str],
    exit_code: Option<i32>,
    success: bool,
    duration_ms: u64,
) {
    if let Some(sink) = input.context.as_ref().and_then(|c| c.event_sink.as_ref()) {
        sink.emit(DomainEvent::new(
            DomainEventKind::PrivilegedCommandExecuted {
                message_id: orka_core::types::MessageId::new(),
                session_id: orka_core::types::SessionId::new(),
                command: command.to_string(),
                args: args.iter().map(ToString::to_string).collect(),
                approval_id: None,
                approved_by: None,
                exit_code,
                success,
                duration_ms,
            },
        ))
        .await;
    }
}

/// Emit a [`DomainEventKind::PrivilegedCommandDenied`] event if an event sink
/// is attached to the skill context.
pub async fn emit_denied(input: &SkillInput, command: &str, args: &[&str], reason: &str) {
    if let Some(sink) = input.context.as_ref().and_then(|c| c.event_sink.as_ref()) {
        sink.emit(DomainEvent::new(DomainEventKind::PrivilegedCommandDenied {
            message_id: orka_core::types::MessageId::new(),
            session_id: orka_core::types::SessionId::new(),
            command: command.to_string(),
            args: args.iter().map(ToString::to_string).collect(),
            reason: reason.to_string(),
        }))
        .await;
    }
}
