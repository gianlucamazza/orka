use std::path::Path;

use crate::soft_skill::{SoftSkill, SoftSkillMeta};

/// Scan a directory for soft skill subdirectories.
///
/// Each subdirectory containing a `SKILL.md` file is loaded as a soft skill.
/// Errors for individual directories are logged as warnings and skipped.
///
/// # Format
/// ```text
/// skills/
/// └── code-review/
///     └── SKILL.md   # YAML frontmatter (name, description, tags) + markdown body
/// ```
pub fn scan_soft_skills(dir: &Path) -> Vec<SoftSkill> {
    let mut skills = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(path = %dir.display(), %e, "failed to read soft skills directory");
            return skills;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let skill_md = path.join("SKILL.md");
        if !skill_md.exists() {
            continue;
        }

        match load_skill_md(&skill_md, &path) {
            Ok(skill) => {
                tracing::info!(name = %skill.meta.name, path = %path.display(), "loaded soft skill");
                skills.push(skill);
            }
            Err(e) => {
                tracing::warn!(path = %skill_md.display(), %e, "failed to load soft skill, skipping");
            }
        }
    }

    skills
}

fn load_skill_md(skill_md: &Path, dir: &Path) -> anyhow::Result<SoftSkill> {
    let content = std::fs::read_to_string(skill_md)?;

    // Parse YAML frontmatter delimited by ---
    let (meta, body) = parse_frontmatter(&content)?;

    Ok(SoftSkill::new(
        meta,
        body.trim().to_string(),
        dir.to_path_buf(),
    ))
}

fn parse_frontmatter(content: &str) -> anyhow::Result<(SoftSkillMeta, String)> {
    // Content must start with ---
    let rest = content
        .strip_prefix("---")
        .ok_or_else(|| anyhow::anyhow!("SKILL.md must start with --- YAML frontmatter"))?;

    // Find closing ---
    let end = rest
        .find("\n---")
        .ok_or_else(|| anyhow::anyhow!("SKILL.md frontmatter not closed with ---"))?;

    let yaml = &rest[..end];
    let body = &rest[end + 4..]; // skip \n---

    let meta: SoftSkillMeta = serde_yml::from_str(yaml)
        .map_err(|e| anyhow::anyhow!("invalid SKILL.md frontmatter: {e}"))?;

    Ok((meta, body.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_frontmatter() {
        let content = r#"---
name: code-review
description: Reviews code quality. Use when asked for code review.
tags: [development]
---
# Code Review
Do the review.
"#;
        let (meta, body) = parse_frontmatter(content).unwrap();
        assert_eq!(meta.name, "code-review");
        assert_eq!(meta.tags, vec!["development"]);
        assert!(body.contains("Code Review"));
    }

    #[test]
    fn parse_missing_delimiter_errors() {
        let content = "name: test\n";
        assert!(parse_frontmatter(content).is_err());
    }
}
