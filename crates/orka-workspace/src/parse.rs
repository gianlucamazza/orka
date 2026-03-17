use serde::de::DeserializeOwned;

#[derive(Debug, Clone)]
pub struct Document<T> {
    pub frontmatter: T,
    pub body: String,
}

pub fn parse_document<T: DeserializeOwned>(raw: &str) -> Result<Document<T>, orka_core::Error> {
    let trimmed = raw.trim_start();

    if !trimmed.starts_with("---") {
        return Err(orka_core::Error::Workspace(
            "missing opening --- delimiter".to_string(),
        ));
    }

    // Skip past the opening ---
    let after_open = &trimmed[3..];
    let after_open = after_open.strip_prefix('\n').unwrap_or(after_open);

    // Find closing ---
    let closing_pos = after_open
        .find("\n---")
        .ok_or_else(|| orka_core::Error::Workspace("missing closing --- delimiter".to_string()))?;

    let yaml_str = &after_open[..closing_pos];
    let rest = &after_open[closing_pos + 4..]; // skip "\n---"
    let body = rest.strip_prefix('\n').unwrap_or(rest).to_string();

    let frontmatter: T = serde_yaml::from_str(yaml_str)
        .map_err(|e| orka_core::Error::Workspace(format!("failed to parse frontmatter: {e}")))?;

    Ok(Document { frontmatter, body })
}

/// Extract body from a markdown file, stripping optional YAML frontmatter.
/// If the file has no frontmatter delimiters, the entire content is returned.
pub fn strip_frontmatter(raw: &str) -> String {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return raw.to_string();
    }
    let after_open = &trimmed[3..];
    let after_open = after_open.strip_prefix('\n').unwrap_or(after_open);
    match after_open.find("\n---") {
        Some(pos) => {
            let rest = &after_open[pos + 4..];
            rest.strip_prefix('\n').unwrap_or(rest).to_string()
        }
        None => raw.to_string(),
    }
}
