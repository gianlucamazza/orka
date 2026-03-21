use serde::Deserialize;
use std::path::PathBuf;

/// YAML frontmatter parsed from a SKILL.md file.
#[derive(Debug, Clone, Deserialize)]
pub struct SoftSkillMeta {
    /// Unique skill name (kebab-case, max 64 chars).
    pub name: String,
    /// Description for LLM-based selection. Should say what the skill does and when to use it.
    pub description: String,
    /// Optional tags for grouping.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// A soft skill loaded from a SKILL.md directory.
///
/// Soft skills are NOT LLM tools — they inject procedural instructions into
/// the agent system prompt when activated for a given request.
#[derive(Debug, Clone)]
pub struct SoftSkill {
    /// Parsed frontmatter (always in memory after loading).
    pub meta: SoftSkillMeta,
    /// Full markdown body of SKILL.md (instructions injected when activated).
    pub body: String,
    /// Root directory of this skill (for resolving auxiliary files).
    pub dir: PathBuf,
}

impl SoftSkill {
    /// Create a new soft skill.
    pub fn new(meta: SoftSkillMeta, body: String, dir: PathBuf) -> Self {
        Self { meta, body, dir }
    }

    /// Short summary for LLM selection prompt.
    pub fn summary_line(&self) -> String {
        format!("- **{}**: {}", self.meta.name, self.meta.description)
    }
}

/// Lightweight handle with only metadata, used for the selection step.
#[derive(Debug, Clone)]
pub struct SoftSkillSummary {
    /// Skill name.
    pub name: String,
    /// Skill description.
    pub description: String,
    /// Optional tags.
    pub tags: Vec<String>,
}

impl From<&SoftSkill> for SoftSkillSummary {
    fn from(s: &SoftSkill) -> Self {
        Self {
            name: s.meta.name.clone(),
            description: s.meta.description.clone(),
            tags: s.meta.tags.clone(),
        }
    }
}
