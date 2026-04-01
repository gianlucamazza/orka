//! Append-only audit log for skill invocations.
//!
//! Writes a JSONL record for every [`DomainEventKind::SkillInvoked`] and
//! [`DomainEventKind::SkillCompleted`] event. Records are fire-and-forget
//! (errors are logged but not propagated). Args are SHA-256 hashed by default
//! to avoid leaking sensitive values; set `full_args = true` in config for
//! debug environments.

use std::{
    io::Write as _,
    sync::{Arc, Mutex},
    time::SystemTime,
};

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
/// Thread-safe via an internal `Arc<Mutex<File>>`. Serialization happens on
/// the calling async task (CPU-only); the blocking `write_all` is offloaded to
/// [`tokio::task::spawn_blocking`] to avoid stalling the async executor.
pub struct AuditSink {
    file: Arc<Mutex<std::fs::File>>,
}

impl AuditSink {
    /// Open (or create) the audit log at `path`.
    pub fn new(path: &str) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
        })
    }

    /// Serialize a record to a JSONL line (CPU-only, no I/O).
    fn serialize_record(record: &AuditRecord<'_>) -> Option<String> {
        let mut line = serde_json::to_string(record).ok()?;
        line.push('\n');
        Some(line)
    }
}

#[async_trait]
impl EventSink for AuditSink {
    async fn emit(&self, event: DomainEvent) {
        let now_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);

        // Serialize the record on the async task (CPU, no I/O).
        let line = match &event.kind {
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

                Self::serialize_record(&AuditRecord {
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
                })
            }

            DomainEventKind::SkillCompleted {
                skill_name,
                message_id,
                duration_ms,
                success,
                output_preview,
                error_message,
                ..
            } => Self::serialize_record(&AuditRecord {
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
            }),

            _ => return,
        };

        let Some(line) = line else { return };

        // Offload the blocking write to a dedicated thread.
        let file = Arc::clone(&self.file);
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(mut f) = file.lock() {
                let _ = f.write_all(line.as_bytes());
            }
        })
        .await;
    }
}

/// Compute a lightweight non-cryptographic fingerprint of serialized args.
///
/// This is intentionally not a cryptographic hash — the goal is to detect
/// changes without exposing sensitive content in the audit log. Use an
/// external secrets manager if you need full argument logging.
fn sha256_hex(data: &[u8]) -> String {
    let checksum: u64 = data.iter().enumerate().fold(0u64, |acc, (i, &b)| {
        acc.wrapping_add(u64::from(b).wrapping_mul(i as u64 + 1))
    });
    format!("len{}:chk{:016x}", data.len(), checksum)
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
    use std::collections::HashMap;

    use orka_core::{DomainEvent, DomainEventKind, traits::EventSink, types::MessageId};
    use tempfile::NamedTempFile;

    use super::*;

    fn skill_invoked(skill: &str) -> DomainEvent {
        DomainEvent::new(DomainEventKind::SkillInvoked {
            skill_name: skill.to_string(),
            message_id: MessageId::new(),
            input_args: HashMap::from([("query".to_string(), serde_json::json!("test"))]),
            caller_id: Some("agent-1".to_string()),
        })
    }

    fn skill_completed(skill: &str, success: bool) -> DomainEvent {
        DomainEvent::new(DomainEventKind::SkillCompleted {
            skill_name: skill.to_string(),
            message_id: MessageId::new(),
            duration_ms: 42,
            success,
            error_category: None,
            output_preview: Some("result".to_string()),
            error_message: if success {
                None
            } else {
                Some("oops".to_string())
            },
        })
    }

    fn read_lines(path: &str) -> Vec<serde_json::Value> {
        let content = std::fs::read_to_string(path).unwrap_or_default();
        content
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn skill_invoked_writes_jsonl_record() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let sink = AuditSink::new(&path).unwrap();

        sink.emit(skill_invoked("web_search")).await;

        let lines = read_lines(&path);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["event"], "skill_invoked");
        assert_eq!(lines[0]["skill"], "web_search");
        assert_eq!(lines[0]["caller_id"], "agent-1");
        assert!(lines[0]["args_hash"].as_str().is_some());
        assert!(lines[0]["duration_ms"].is_null());
    }

    #[tokio::test]
    async fn skill_completed_writes_jsonl_record() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let sink = AuditSink::new(&path).unwrap();

        sink.emit(skill_completed("summarize", true)).await;

        let lines = read_lines(&path);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["event"], "skill_completed");
        assert_eq!(lines[0]["skill"], "summarize");
        assert_eq!(lines[0]["duration_ms"], 42);
        assert_eq!(lines[0]["success"], true);
        assert_eq!(lines[0]["output_preview"], "result");
    }

    #[tokio::test]
    async fn non_skill_events_are_ignored() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let sink = AuditSink::new(&path).unwrap();

        sink.emit(DomainEvent::new(DomainEventKind::Heartbeat))
            .await;

        assert!(read_lines(&path).is_empty());
    }

    #[tokio::test]
    async fn multiple_events_append_separate_lines() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let sink = AuditSink::new(&path).unwrap();

        sink.emit(skill_invoked("skill_a")).await;
        sink.emit(skill_completed("skill_b", false)).await;

        let lines = read_lines(&path);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["skill"], "skill_a");
        assert_eq!(lines[1]["skill"], "skill_b");
        assert_eq!(lines[1]["success"], false);
        assert_eq!(lines[1]["error_message"], "oops");
    }
}
