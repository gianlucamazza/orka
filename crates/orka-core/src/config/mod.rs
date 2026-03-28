//! Shared configuration submodules for Orka runtime crates.
//!
//! This module contains reusable config primitives and shared runtime sections.
//! The composed top-level `OrkaConfig` schema, config loading, and validation
//! orchestration live in the `orka-config` crate.
//!
//! Shared config in this module is divided into submodules by domain:
//!
//! - [`server`]: HTTP server bind configuration.
//! - [`infrastructure`]: Redis, message bus, queue, session, and memory stores.
//! - [`agent`]: Per-agent runtime and graph configuration.
//! - [`security`]: Authentication, secrets, and sandboxing.
//! - [`observability`]: Metrics, tracing, and audit logging.
//! - [`system`]: Worker, logging, and scheduler configuration.

pub mod agent;
pub mod chart;
pub mod defaults;
pub mod experience;
pub mod http;
pub mod infrastructure;
pub mod observability;
pub mod primitives;
pub mod prompts;
pub mod protocols;
pub mod research;
pub mod security;
pub mod server;
pub mod system;
pub mod tools;

pub use self::{
    agent::*, chart::*, experience::*, http::*, infrastructure::*, observability::*, primitives::*,
    prompts::*, protocols::*, research::*, security::*, server::*, system::*, tools::*,
};
