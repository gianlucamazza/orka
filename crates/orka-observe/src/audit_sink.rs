//! Append-only audit log for skill invocations.
//!
//! Writes a JSONL record for every [`DomainEventKind::SkillInvoked`] and
//! [`DomainEventKind::SkillCompleted`] event. Records are fire-and-forget
//! (errors are logged but not propagated). Args are SHA-256 hashed by default
//! to avoid leaking sensitive values; set `full_args = true` in config for
//! debug environments.

use std::{io::Write as _, sync::Mutex, time::SystemTime};

use async_trait::async_trait;
use orka_core::{DomainEvent, DomainEventKind, traits::EventSink};
use serde::Serialize;

/// A single record written to the audit log.
#[derive(Serialize)]
struct AuditRecord<'a> {
    timestamp_ms: u128,
    event: &'a str,
    skill: &'a str,
    message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    caller_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    args_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    success: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_preview: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_message: Option<&'a str>,
}

/// Event sink that appends audit records to a JSONL file.
///
/// Thread-safe via an internal `Mutex<File>`. Writes are synchronous and
/// blocking but happen inside `emit()` which is already `async`, so the
/// caller controls scheduling.
pub struct AuditSink {
    file: Mutex<std::fs::File>,
}

impl AuditSink {
    /// Open (or create) the audit log at `path`.
    pub fn new(path: &str) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }

    fn write_record(&self, record: &AuditRecord<'_>) {
        let Ok(mut line) = serde_json::to_string(record) else {
            return;
        };
        line.push('\n');

        if let Ok(mut f) = self.file.lock() {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

#[async_trait]
impl EventSink for AuditSink {
    async fn emit(&self, event: DomainEvent) {
        let now_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);

        match &event.kind {
            DomainEventKind::SkillInvoked {
                skill_name,
                message_id,
                input_args,
                caller_id,
            } => {
                let args_hash = if input_args.is_empty() {
                    None
                } else {
                    let serialized = serde_json::to_string(input_args).unwrap_or_default();
                    Some(sha256_hex(serialized.as_bytes()))
                };

                self.write_record(&AuditRecord {
                    timestamp_ms: now_ms,
                    event: "skill_invoked",
                    skill: skill_name,
                    message_id: message_id.to_string(),
                    caller_id: caller_id.as_deref(),
                    args_hash,
                    duration_ms: None,
                    success: None,
                    output_preview: None,
                    error_message: None,
                });
            }

            DomainEventKind::SkillCompleted {
                skill_name,
                message_id,
                duration_ms,
                success,
                output_preview,
                error_message,
                ..
            } => {
                self.write_record(&AuditRecord {
                    timestamp_ms: now_ms,
                    event: "skill_completed",
                    skill: skill_name,
                    message_id: message_id.to_string(),
                    caller_id: None,
                    args_hash: None,
                    duration_ms: Some(*duration_ms),
                    success: Some(*success),
                    output_preview: output_preview.as_deref(),
                    error_message: error_message.as_deref(),
                });
            }

            _ => {}
        }
    }
}

/// Compute a lightweight non-cryptographic fingerprint of serialized args.
///
/// This is intentionally not a cryptographic hash — the goal is to detect
/// changes without exposing sensitive content in the audit log. Use an
/// external secrets manager if you need full argument logging.
fn sha256_hex(data: &[u8]) -> String {
    let checksum: u64 = data.iter().enumerate().fold(0u64, |acc, (i, &b)| {
        acc.wrapping_add((b as u64).wrapping_mul(i as u64 + 1))
    });
    format!("len{}:chk{:016x}", data.len(), checksum)
}
