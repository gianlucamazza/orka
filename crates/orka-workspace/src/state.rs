use crate::config::*;
use crate::parse::Document;

#[derive(Debug, Clone, Default)]
pub struct WorkspaceState {
    pub soul: Option<Document<SoulFrontmatter>>,
    pub tools: Option<Document<ToolsFrontmatter>>,
    pub identity: Option<Document<IdentityFrontmatter>>,
    pub heartbeat: Option<Document<HeartbeatFrontmatter>>,
    pub memory: Option<Document<MemoryFrontmatter>>,
}
