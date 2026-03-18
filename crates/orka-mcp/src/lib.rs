//! Model Context Protocol (MCP) client and server implementation.
//!
//! - [`McpClient`] — connects to external MCP tool servers
//! - [`McpToolBridge`] — adapts MCP tools as Orka [`Skill`](orka_core::traits::Skill) instances
//! - [`McpServer`] — exposes Orka skills as an MCP-compatible endpoint

#![warn(missing_docs)]

/// Skill adapter that wraps an MCP tool as an Orka skill.
mod bridge;
/// JSON-RPC 2.0 client for communicating with an MCP server process over stdio.
mod client;
/// Configuration types for MCP server processes.
mod config;
/// MCP server exposing Orka skills as JSON-RPC 2.0 tools.
pub mod server;
/// Axum HTTP handler for the MCP endpoint.
pub mod transport;

pub use bridge::McpToolBridge;
pub use client::{McpClient, McpContent, McpToolInfo, McpToolResult};
pub use config::McpServerConfig;
pub use server::McpServer;
pub use transport::{McpServerState, handle_mcp_post};
