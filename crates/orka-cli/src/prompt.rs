use std::path::Path;

use colored::Colorize;

/// Build the shell prompt: `~/Workspace/orka (main) ▶ `
pub fn build_prompt(cwd: &Path, last_exit: Option<i32>) -> String {
    let dir = shorten_path(cwd);
    let branch = git_branch(cwd);
    let indicator = match last_exit {
        Some(0) | None => "▶".green().to_string(),
        Some(_) => "▶".red().to_string(),
    };
    if branch.is_empty() {
        format!("{dir} {indicator} ")
    } else {
        format!("{dir} ({branch}) {indicator} ", branch = branch.dimmed())
    }
}

/// Shorten a path by replacing $HOME with `~`.
fn shorten_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        if rest.as_os_str().is_empty() {
            return "~".to_string();
        }
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

/// Get the current git branch name, or empty string if not in a repo.
fn git_branch(cwd: &Path) -> String {
    // Fast path: read .git/HEAD directly
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        let head = d.join(".git/HEAD");
        if head.exists()
            && let Ok(contents) = std::fs::read_to_string(&head)
        {
            let contents = contents.trim();
            if let Some(refname) = contents.strip_prefix("ref: refs/heads/") {
                return refname.to_string();
            }
            // Detached HEAD — show short hash
            return contents.chars().take(7).collect();
        }
        dir = d.parent();
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn shorten_home_path() {
        if let Some(home) = dirs::home_dir() {
            let path = home.join("projects/foo");
            assert_eq!(shorten_path(&path), "~/projects/foo");
        }
    }

    #[test]
    fn shorten_non_home_path() {
        let path = PathBuf::from("/tmp/test");
        assert_eq!(shorten_path(&path), "/tmp/test");
    }

    #[test]
    fn shorten_home_itself() {
        if let Some(home) = dirs::home_dir() {
            assert_eq!(shorten_path(&home), "~");
        }
    }

    #[test]
    fn git_branch_outside_repo() {
        // /tmp is unlikely to be a git repo
        assert_eq!(git_branch(Path::new("/tmp")), "");
    }

    #[test]
    fn prompt_success_exit() {
        let p = build_prompt(Path::new("/tmp"), Some(0));
        assert!(p.contains("/tmp"));
        assert!(p.contains("▶"));
    }

    #[test]
    fn prompt_no_exit_yet() {
        let p = build_prompt(Path::new("/tmp"), None);
        assert!(p.contains("▶"));
    }
}
