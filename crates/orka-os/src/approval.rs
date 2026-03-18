use async_trait::async_trait;
use chrono::{DateTime, Utc};
use orka_core::types::{MessageId, SessionId};
use uuid::Uuid;

/// A request for human approval before executing a privileged command.
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    /// Unique identifier for this approval request.
    pub id: Uuid,
    /// The command to be executed (e.g. `"systemctl"`).
    pub command: String,
    /// Arguments to be passed to the command.
    pub args: Vec<String>,
    /// Human-readable explanation of why the command needs to run.
    pub reason: String,
    /// Session in which the command was requested.
    pub session_id: SessionId,
    /// Message that triggered the command request.
    pub message_id: MessageId,
    /// When the approval request was created.
    pub requested_at: DateTime<Utc>,
    /// When the approval request will expire if unanswered.
    pub expires_at: DateTime<Utc>,
}

/// The decision returned by an approval channel.
#[derive(Debug, Clone)]
pub enum ApprovalDecision {
    /// The operator approved the command.
    Approved,
    /// The operator denied the command, with an optional reason.
    Denied {
        /// Human-readable reason for the denial.
        reason: String,
    },
    /// No response was received before the deadline.
    Expired,
}

/// Channel through which privileged command approvals are requested.
#[async_trait]
pub trait ApprovalChannel: Send + Sync + 'static {
    /// Submit an approval request and wait for a decision.
    async fn request_approval(&self, req: ApprovalRequest) -> orka_core::Result<ApprovalDecision>;
}

/// Auto-approve channel for use when `require_confirmation` is false.
pub struct AutoApproveChannel;

#[async_trait]
impl ApprovalChannel for AutoApproveChannel {
    async fn request_approval(&self, _req: ApprovalRequest) -> orka_core::Result<ApprovalDecision> {
        Ok(ApprovalDecision::Approved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_request_fields() {
        let req = ApprovalRequest {
            id: Uuid::now_v7(),
            command: "systemctl".into(),
            args: vec!["restart".into(), "nginx".into()],
            reason: "restart web server".into(),
            session_id: SessionId::new(),
            message_id: MessageId::new(),
            requested_at: Utc::now(),
            expires_at: Utc::now(),
        };
        assert_eq!(req.command, "systemctl");
    }

    #[tokio::test]
    async fn auto_approve_channel_approves() {
        let channel = AutoApproveChannel;
        let req = ApprovalRequest {
            id: Uuid::now_v7(),
            command: "test".into(),
            args: vec![],
            reason: "test".into(),
            session_id: SessionId::new(),
            message_id: MessageId::new(),
            requested_at: Utc::now(),
            expires_at: Utc::now(),
        };
        let decision = channel.request_approval(req).await.unwrap();
        assert!(matches!(decision, ApprovalDecision::Approved));
    }
}
