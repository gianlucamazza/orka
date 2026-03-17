use crate::config::*;
use crate::parse::Document;

#[derive(Debug, Clone, Default)]
pub struct WorkspaceState {
    pub soul: Option<Document<SoulFrontmatter>>,
    pub tools_body: Option<String>,
}
