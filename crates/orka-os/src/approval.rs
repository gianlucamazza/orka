use async_trait::async_trait;
use chrono::{DateTime, Utc};
use orka_core::types::{MessageId, SessionId};
use uuid::Uuid;

/// A request for human approval before executing a privileged command.
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub id: Uuid,
    pub command: String,
    pub args: Vec<String>,
    pub reason: String,
    pub session_id: SessionId,
    pub message_id: MessageId,
    pub requested_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// The decision returned by an approval channel.
#[derive(Debug, Clone)]
pub enum ApprovalDecision {
    Approved,
    Denied { reason: String },
    Expired,
}

/// Channel through which privileged command approvals are requested.
#[async_trait]
pub trait ApprovalChannel: Send + Sync + 'static {
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
