//! Agent-to-Agent (A2A) protocol support for inter-agent communication.
//!
//! - [`build_agent_card`] — generates an A2A agent card from workspace config
//! - [`a2a_router`] — axum router exposing A2A endpoints

#![warn(missing_docs)]

#[allow(missing_docs)]
pub mod agent_card;
#[allow(missing_docs)]
pub mod routes;
#[allow(missing_docs)]
pub mod types;

pub use agent_card::build_agent_card;
pub use routes::{A2aState, a2a_router};
pub use types::{
    A2aMessage, AgentCapabilities, AgentCard, AgentSkill, Artifact, AuthConfig, MessagePart, Task,
    TaskStatus,
};
