use crate::config::*;
use crate::parse::Document;
use orka_prompts::template::TemplateRegistry;
use std::sync::Arc;

/// Live state of a loaded workspace.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceState {
    /// Parsed `SOUL.md` document (frontmatter + body).
    pub soul: Option<Document<SoulFrontmatter>>,
    /// Raw markdown body from `TOOLS.md`, if present.
    pub tools_body: Option<String>,
    /// Template registry for prompt rendering.
    pub templates: Option<Arc<TemplateRegistry>>,
}
