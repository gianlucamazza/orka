//! Workspace configuration, loading, and hot-reload watching.
//!
//! - [`WorkspaceLoader`] — loads workspace definitions from TOML files
//! - [`WorkspaceRegistry`] — thread-safe registry of active workspaces
//! - [`WorkspaceWatcher`] — watches the filesystem for workspace changes

#![warn(missing_docs)]

#[allow(missing_docs)]
pub mod config;
#[allow(missing_docs)]
pub mod loader;
#[allow(missing_docs)]
pub mod parse;
#[allow(missing_docs)]
pub mod registry;
#[allow(missing_docs)]
pub mod state;
#[allow(missing_docs)]
pub mod watcher;

pub use config::SoulFrontmatter;
pub use loader::{WorkspaceEvent, WorkspaceLoader};
pub use parse::{Document, strip_frontmatter};
pub use registry::WorkspaceRegistry;
pub use state::WorkspaceState;
pub use watcher::WorkspaceWatcher;
