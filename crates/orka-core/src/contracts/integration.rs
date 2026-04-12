//! Integration classification and trust model.
//!
//! Formalizes the four categories of integrations in Orka and the trust level
//! associated with each. These are used by the gateway to enforce auth policy
//! and by the info endpoint to expose integration metadata.

use serde::{Deserialize, Serialize};

/// The category of an integration surface.
///
/// Each category has different UX expectations, auth requirements, and
/// capability profiles, but all share the same canonical interaction contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum IntegrationClass {
    /// First-party client applications (mobile app, web app, desktop app).
    ///
    /// Examples: mobile router, future web client.
    ProductClient,

    /// Third-party messaging platforms integrated via webhook or bot API.
    ///
    /// Examples: Telegram, Discord, Slack, `WhatsApp`.
    MessagingChannel,

    /// Operational tooling with privileged access to the system.
    ///
    /// Examples: CLI, admin dashboard.
    OperationalClient,

    /// Federated protocol peers for agent-to-agent or tool communication.
    ///
    /// Examples: MCP, A2A.
    FederationProtocol,
}

/// The trust level granted to messages from an integration.
///
/// Used by the gateway to validate that claimed origins match auth evidence
/// and to enforce capability-based access control.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    utoipa::ToSchema,
)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// User-authenticated via JWT with device identity.
    ///
    /// Highest trust; grants access to all declared capabilities.
    UserAuthenticated,

    /// Message authenticated via platform-signed webhook (HMAC-SHA256).
    ///
    /// Trusted for the declared messaging channel capabilities only.
    VerifiedWebhook,

    /// Authenticated via long-lived bot token.
    ///
    /// Trusted for the declared bot capabilities only.
    BotToken,

    /// Trusted operator with API key access.
    ///
    /// Grants operational capabilities without user identity.
    TrustedOperator,

    /// Federated peer authenticated via OAuth 2.1 or per-task credentials.
    FederatedPeer,
}
