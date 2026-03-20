use std::path::Path;

use colored::Colorize;

/// Build the shell prompt: `~/Workspace/orka (main) ❯ `
///
/// Returns `(plain, colored)` where `plain` has no ANSI codes (passed to
/// `readline()` for correct width calculation) and `colored` is the styled
/// version returned by `Highlighter::highlight_prompt`.
pub fn build_prompt(cwd: &Path, last_exit: Option<i32>) -> (String, String) {
    let dir = shorten_path(cwd);
    let branch = git_branch(cwd);
    let (plain, colored) = if branch.is_empty() {
        let indicator_plain = "❯";
        let indicator_colored = match last_exit {
            Some(0) | None => "❯".green().to_string(),
            Some(_) => "❯".red().to_string(),
        };
        (
            format!("{dir} {indicator_plain} "),
            format!("{dir} {indicator_colored} "),
        )
    } else {
        let indicator_plain = "❯";
        let indicator_colored = match last_exit {
            Some(0) | None => "❯".green().to_string(),
            Some(_) => "❯".red().to_string(),
        };
        let branch_colored = branch.dimmed().to_string();
        (
            format!("{dir} ({branch}) {indicator_plain} "),
            format!("{dir} ({branch_colored}) {indicator_colored} "),
        )
    };
    (plain, colored)
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
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        let git_path = d.join(".git");
        let head_path = if git_path.is_dir() {
            // Normal repository
            git_path.join("HEAD")
        } else if git_path.is_file() {
            // Git worktree: .git is a file containing "gitdir: <path>"
            let raw = std::fs::read_to_string(&git_path).unwrap_or_default();
            if let Some(gitdir) = raw.trim().strip_prefix("gitdir: ") {
                // Resolve relative gitdir paths relative to the directory containing
                // the .git file, not the process CWD.
                let gitdir_path = std::path::PathBuf::from(gitdir);
                let resolved = if gitdir_path.is_absolute() {
                    gitdir_path
                } else {
                    d.join(gitdir_path)
                };
                resolved.join("HEAD")
            } else {
                dir = d.parent();
                continue;
            }
        } else {
            dir = d.parent();
            continue;
        };

        if let Ok(contents) = std::fs::read_to_string(&head_path) {
            let contents = contents.trim();
            if let Some(refname) = contents.strip_prefix("ref: refs/heads/") {
                return refname.to_string();
            }
            // Detached HEAD — show "detached:" prefix with short hash
            return format!("detached:{}", contents.chars().take(7).collect::<String>());
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
        let (plain, colored) = build_prompt(Path::new("/tmp"), Some(0));
        assert!(plain.contains("/tmp"));
        assert!(plain.contains("❯"));
        // Plain prompt must not contain any ANSI escape sequences
        assert!(!plain.contains('\x1b'));
        // Colored prompt must contain the indicator (ANSI may be stripped in non-TTY tests)
        assert!(colored.contains("❯"));
    }

    #[test]
    fn prompt_no_exit_yet() {
        let (plain, colored) = build_prompt(Path::new("/tmp"), None);
        assert!(plain.contains("❯"));
        assert!(!plain.contains('\x1b'));
        assert!(colored.contains("❯"));
    }

    #[test]
    fn prompt_plain_has_no_ansi() {
        let (plain, _) = build_prompt(Path::new("/tmp"), Some(1));
        assert!(!plain.contains('\x1b'), "plain prompt must not contain ANSI escapes");
    }
}
