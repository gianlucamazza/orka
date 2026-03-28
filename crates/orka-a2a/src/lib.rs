//! Agent-to-Agent (A2A) protocol support for inter-agent communication.
//!
//! Implements the [A2A protocol v1.0](https://a2a-protocol.org/latest/specification/)
//! as an Axum router that can be mounted into any Orka server instance.
//!
//! ## Entry points
//!
//! - [`build_agent_card`] — generate an [`AgentCard`] from the live skill
//!   registry
//! - [`a2a_router`] — axum router exposing `GET /.well-known/agent.json` and
//!   `POST /a2a`
//! - [`InMemoryTaskStore`] / [`RedisTaskStore`] — pluggable task backends

#![warn(missing_docs)]

/// Agent card builder — constructs the `/.well-known/agent.json` payload.
pub mod agent_card;
/// A2A configuration types.
pub mod config;
/// Outbound A2A discovery client and in-memory agent directory.
pub mod discovery;
/// JSON-RPC and A2A-specific error types.
pub mod error;
/// Typed JSON-RPC 2.0 envelope types.
pub mod jsonrpc;
/// Push notification configuration backends.
pub mod push_store;
/// Axum route handlers for A2A JSON-RPC endpoints.
pub mod routes;
/// Task persistence backends (`InMemoryTaskStore`, `RedisTaskStore`).
pub mod store;
/// A2A protocol data types (tasks, messages, artifacts, agent card, events).
pub mod types;
/// Outbound webhook delivery for push notifications.
pub mod webhook;

pub use agent_card::{build_agent_card, build_agent_card_with_auth};
pub use config::A2aConfig;
pub use discovery::{AgentDirectory, DiscoveryClient};
pub use error::A2aError;
pub use push_store::{
    InMemoryPushNotificationStore, PushNotificationStore, RedisPushNotificationStore,
};
pub use routes::{A2aState, a2a_router, a2a_routes_split};
pub use store::{InMemoryTaskStore, RedisTaskStore, TaskStore};
pub use types::{
    AgentCard, AgentSkill, Artifact, FileContent, InterfaceCapabilities, ListTasksParams,
    ListTasksResult, Message, MessageKind, Part, PushNotificationAuth, PushNotificationConfig,
    Role, SecurityScheme, SkillSecurity, SupportedInterface, Task, TaskEvent, TaskKind, TaskState,
    TaskStatus,
};
pub use webhook::WebhookDeliverer;
