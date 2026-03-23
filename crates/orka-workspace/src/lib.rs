//! Workspace configuration, loading, and hot-reload watching.
//!
//! - [`WorkspaceLoader`] — loads workspace definitions from TOML files
//! - [`WorkspaceRegistry`] — thread-safe registry of active workspaces
//! - [`WorkspaceWatcher`] — watches the filesystem for workspace changes

#![warn(missing_docs)]

use orka_prompts::template::TemplateRegistry;

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

/// Load built-in templates into the registry.
pub async fn load_builtins(registry: &TemplateRegistry) -> Result<(), orka_prompts::template::TemplateError> {
    registry.register_inline(
        "system/reflection",
        include_str!("../../orka-prompts/templates/system/reflection.hbs"),
    ).await?;

    registry.register_inline(
        "system/distillation",
        include_str!("../../orka-prompts/templates/system/distillation.hbs"),
    ).await?;

    registry.register_inline(
        "sections/persona",
        include_str!("../../orka-prompts/templates/sections/persona.hbs"),
    ).await?;

    registry.register_inline(
        "sections/datetime",
        include_str!("../../orka-prompts/templates/sections/datetime.hbs"),
    ).await?;

    registry.register_inline(
        "sections/tools",
        include_str!("../../orka-prompts/templates/sections/tools.hbs"),
    ).await?;

    registry.register_inline(
        "sections/workspace",
        include_str!("../../orka-prompts/templates/sections/workspace.hbs"),
    ).await?;

    registry.register_inline(
        "sections/principles",
        include_str!("../../orka-prompts/templates/sections/principles.hbs"),
    ).await?;

    registry.register_inline(
        "sections/summary",
        include_str!("../../orka-prompts/templates/sections/summary.hbs"),
    ).await?;

    registry.register_inline(
        "selection/soft_skill",
        include_str!("../../orka-prompts/templates/selection/soft_skill.hbs"),
    ).await?;

    Ok(())
}
