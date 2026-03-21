use std::collections::HashMap;

use crate::soft_skill::{SoftSkill, SoftSkillSummary};

/// Registry for instruction-based soft skills.
///
/// Soft skills are NOT LLM tools. They inject procedural instructions into
/// the agent system prompt when activated for a given request.
pub struct SoftSkillRegistry {
    skills: HashMap<String, SoftSkill>,
}

impl SoftSkillRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
        }
    }

    /// Register a soft skill, replacing any existing skill with the same name.
    pub fn register(&mut self, skill: SoftSkill) {
        self.skills.insert(skill.meta.name.clone(), skill);
    }

    /// Return a summary of all registered soft skills (for LLM selection).
    pub fn summaries(&self) -> Vec<SoftSkillSummary> {
        self.skills.values().map(SoftSkillSummary::from).collect()
    }

    /// Get a soft skill by name.
    pub fn get(&self, name: &str) -> Option<&SoftSkill> {
        self.skills.get(name)
    }

    /// List all skill names.
    pub fn list(&self) -> Vec<&str> {
        self.skills.keys().map(String::as_str).collect()
    }

    /// Return true if no skills are registered.
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// Return the number of registered skills.
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    /// Build a prompt section from the given skill names.
    ///
    /// Returns a markdown-formatted string to inject into the system prompt.
    /// Skills not found in the registry are silently skipped.
    pub fn build_prompt_section(&self, names: &[&str]) -> String {
        let mut out = String::from("## Active Skills\n\n");
        let mut any = false;

        for name in names {
            if let Some(skill) = self.skills.get(*name) {
                out.push_str(&format!("### {}\n\n", skill.meta.name));
                out.push_str(&skill.body);
                out.push_str("\n\n");
                any = true;
            }
        }

        if any { out } else { String::new() }
    }

    /// Build a compact selection prompt listing all skill summaries.
    ///
    /// Used to ask the LLM which skills are relevant for a given request.
    pub fn build_selection_prompt(&self) -> String {
        let mut out = String::from(
            "The following instruction skills are available. \
             Reply with a JSON array of skill names that are relevant to the user's request. \
             Reply with an empty array `[]` if none apply.\n\n",
        );
        for skill in self.skills.values() {
            out.push_str(&skill.summary_line());
            out.push('\n');
        }
        out
    }
}

impl Default for SoftSkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soft_skill::SoftSkillMeta;
    use std::path::PathBuf;

    fn make_skill(name: &str, desc: &str, body: &str) -> SoftSkill {
        SoftSkill::new(
            SoftSkillMeta {
                name: name.to_string(),
                description: desc.to_string(),
                tags: vec![],
            },
            body.to_string(),
            PathBuf::from("."),
        )
    }

    #[test]
    fn register_and_get() {
        let mut reg = SoftSkillRegistry::new();
        reg.register(make_skill("test", "A test skill", "## Test\nDo stuff."));
        assert!(reg.get("test").is_some());
        assert!(reg.get("other").is_none());
    }

    #[test]
    fn build_prompt_section_includes_body() {
        let mut reg = SoftSkillRegistry::new();
        reg.register(make_skill(
            "review",
            "Code review",
            "## Review\nCheck quality.",
        ));
        let section = reg.build_prompt_section(&["review"]);
        assert!(section.contains("## Active Skills"));
        assert!(section.contains("Check quality."));
    }

    #[test]
    fn build_prompt_section_empty_for_unknown() {
        let reg = SoftSkillRegistry::new();
        let section = reg.build_prompt_section(&["nonexistent"]);
        assert!(section.is_empty());
    }
}
