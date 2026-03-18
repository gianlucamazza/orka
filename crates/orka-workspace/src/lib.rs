//! Workspace configuration, loading, and hot-reload watching.
//!
//! - [`WorkspaceLoader`] — loads workspace definitions from TOML files
//! - [`WorkspaceRegistry`] — thread-safe registry of active workspaces
//! - [`WorkspaceWatcher`] — watches the filesystem for workspace changes

#![warn(missing_docs)]

/// SOUL frontmatter type parsed from workspace YAML.
pub mod config;
/// Workspace file loader with change-event broadcasting.
pub mod loader;
/// Markdown document parser that handles YAML frontmatter.
pub mod parse;
/// Multi-workspace registry for named workspace lookups.
pub mod registry;
/// Live workspace state (parsed SOUL + TOOLS content).
pub mod state;
/// Filesystem watcher that hot-reloads workspace files.
pub mod watcher;

pub use config::SoulFrontmatter;
pub use loader::{WorkspaceEvent, WorkspaceLoader};
pub use parse::{Document, strip_frontmatter};
pub use registry::WorkspaceRegistry;
pub use state::WorkspaceState;
pub use watcher::WorkspaceWatcher;
