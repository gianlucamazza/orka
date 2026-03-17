//! Model Context Protocol (MCP) client and server implementation.
//!
//! - [`McpClient`] — connects to external MCP tool servers
//! - [`McpToolBridge`] — adapts MCP tools as Orka [`Skill`](orka_core::traits::Skill) instances
//! - [`McpServer`] — exposes Orka skills as an MCP-compatible endpoint

#![warn(missing_docs)]

#[allow(missing_docs)]
mod bridge;
#[allow(missing_docs)]
mod client;
#[allow(missing_docs)]
mod config;
#[allow(missing_docs)]
pub mod server;
#[allow(missing_docs)]
pub mod transport;

pub use bridge::McpToolBridge;
pub use client::{McpClient, McpContent, McpToolInfo, McpToolResult};
pub use config::McpServerConfig;
pub use server::McpServer;
pub use transport::{McpServerState, handle_mcp_post};
