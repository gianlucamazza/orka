//! Shared configuration submodules for Orka runtime crates.
//!
//! This module contains reusable config primitives and shared runtime sections.
//! The composed top-level `OrkaConfig` schema, config loading, and validation
//! orchestration live in the `orka-config` crate.
//!
//! Shared config in this module is divided into submodules by domain:
//!
//! - [`agent`]: Per-agent runtime and graph configuration.
//!
//! Domain-owned runtime config such as OS integration, scheduler, HTTP,
//! prompts, research, chart, experience, MCP, A2A, guardrails, tools, bus,
//! session, memory, observe, and secrets now lives in the owning crates.
//! Top-level composed config such as server, logging, redis, worker, and queue
//! policy lives in `orka-config`.

pub mod agent;
pub mod defaults;
pub mod primitives;

pub use self::{agent::*, primitives::*};
