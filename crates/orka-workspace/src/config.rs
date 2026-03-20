use serde::{Deserialize, Serialize};

/// YAML frontmatter parsed from a `SOUL.md` file.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SoulFrontmatter {
    /// Agent name (overrides the workspace directory name when set).
    #[serde(default)]
    pub name: Option<String>,
    /// Semantic version string for the SOUL definition.
    #[serde(default)]
    pub version: Option<String>,
    /// One-line description of the agent's purpose.
    #[serde(default)]
    pub description: Option<String>,
}
