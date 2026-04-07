//! Canonical contracts, capability model, and interaction types for Orka.
//!
//! This crate defines the shared contract layer that all integration surfaces —
//! messaging channels, product clients, operational clients, and federation
//! protocols — must conform to. It is intentionally a thin, low-dependency
//! crate so that adapters can depend on it without pulling in the full
//! `orka-core` machinery.
//!
//! # Key types
//!
//! - **[`Capability`] / [`CapabilitySet`]**: declarative capability model for
//!   each integration surface
//! - **[`InboundInteraction`] / [`OutboundInteraction`]**: canonical message
//!   types that replace the free-form `Envelope` + metadata bag at the adapter
//!   boundary
//! - **[`PlatformContext`] / [`SenderInfo`]**: two-level metadata replacing
//!   scattered string keys
//! - **[`RealtimeEvent`]**: unified streaming event schema for SSE, WebSocket,
//!   and future transports
//! - **[`IntegrationClass`] / [`TrustLevel`]**: formal classification and auth
//!   trust model

pub mod capability;
pub mod event;
pub mod integration;
pub mod interaction;
pub mod platform;

pub use capability::{Capability, CapabilitySet};
pub use event::RealtimeEvent;
pub use integration::{IntegrationClass, TrustLevel};
pub use interaction::{
    CommandContent, EventContent, InboundInteraction, InteractionContent, MediaAttachment,
    OutboundInteraction, RichInput, TraceContext,
};
pub use platform::{PlatformContext, SenderInfo};
