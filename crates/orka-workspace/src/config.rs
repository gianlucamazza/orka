use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SoulFrontmatter {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}
