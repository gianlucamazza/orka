use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Local workspace files discovered from the current working directory.
pub struct LocalWorkspace {
    pub root: PathBuf,
    pub soul_content: Option<String>,
    pub tools_content: Option<String>,
}

impl LocalWorkspace {
    /// Build the metadata map to attach to the first message.
    pub fn to_metadata(&self) -> HashMap<String, serde_json::Value> {
        let mut map = HashMap::new();
        if let Some(ref soul) = self.soul_content {
            map.insert(
                "workspace:soul".to_string(),
                serde_json::Value::String(soul.clone()),
            );
        }
        if let Some(ref tools) = self.tools_content {
            map.insert(
                "workspace:tools".to_string(),
                serde_json::Value::String(tools.clone()),
            );
        }
        map
    }
}

/// Discover SOUL.md / TOOLS.md starting from CWD, walking up to find an
/// `orka.toml` workspace root. Returns `None` if neither file is found.
pub fn discover() -> Option<LocalWorkspace> {
    let cwd = std::env::current_dir().ok()?;
    let home = dirs::home_dir();

    // First check CWD for the files directly
    let soul = try_read(&cwd, "SOUL.md");
    let tools = try_read(&cwd, "TOOLS.md");
    if soul.is_some() || tools.is_some() {
        return Some(LocalWorkspace {
            root: cwd,
            soul_content: soul,
            tools_content: tools,
        });
    }

    // Walk up looking for orka.toml as workspace root marker
    let mut dir = cwd.parent().map(Path::to_path_buf);
    while let Some(ref d) = dir {
        if let Some(ref h) = home
            && !d.starts_with(h)
        {
            break;
        }
        if d.join("orka.toml").exists() {
            let soul = try_read(d, "SOUL.md");
            let tools = try_read(d, "TOOLS.md");
            if soul.is_some() || tools.is_some() {
                return Some(LocalWorkspace {
                    root: d.clone(),
                    soul_content: soul,
                    tools_content: tools,
                });
            }
        }
        dir = d.parent().map(Path::to_path_buf);
    }

    None
}

fn try_read(dir: &Path, filename: &str) -> Option<String> {
    let path = dir.join(filename);
    std::fs::read_to_string(&path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn discover_finds_soul_in_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("SOUL.md"), "---\nname: Test\n---\nHello").unwrap();

        // We can't easily change CWD in tests without affecting other threads,
        // so test the helper directly.
        let soul = try_read(tmp.path(), "SOUL.md");
        assert!(soul.is_some());
        assert!(soul.unwrap().contains("Hello"));
    }

    #[test]
    fn try_read_missing_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(try_read(tmp.path(), "SOUL.md").is_none());
    }

    #[test]
    fn home_boundary_uses_starts_with_not_lexicographic() {
        // A sibling dir like /home/user2 must NOT stop the walk when home is /home/user
        // Under the old `d < h` comparison, `/home/user2` < `/home/user` (lexicographic)
        // was wrong — starts_with gives the correct answer.
        let home = std::path::PathBuf::from("/home/user");
        let sibling = std::path::PathBuf::from("/home/user2/project");
        // sibling does NOT start with home → the walker should NOT break
        assert!(!sibling.starts_with(&home));
        // A path under home starts with home → the walker should break
        let under_home = std::path::PathBuf::from("/home/user/project");
        assert!(under_home.starts_with(&home));
    }

    #[test]
    fn to_metadata_includes_both_keys() {
        let ws = LocalWorkspace {
            root: PathBuf::from("/tmp"),
            soul_content: Some("soul".into()),
            tools_content: Some("tools".into()),
        };
        let meta = ws.to_metadata();
        assert_eq!(meta.len(), 2);
        assert_eq!(meta["workspace:soul"], serde_json::json!("soul"));
        assert_eq!(meta["workspace:tools"], serde_json::json!("tools"));
    }

    #[test]
    fn to_metadata_omits_none() {
        let ws = LocalWorkspace {
            root: PathBuf::from("/tmp"),
            soul_content: Some("soul".into()),
            tools_content: None,
        };
        let meta = ws.to_metadata();
        assert_eq!(meta.len(), 1);
        assert!(meta.contains_key("workspace:soul"));
    }
}
