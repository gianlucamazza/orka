use std::path::Path;

use colored::Colorize;

/// Wrap ANSI escape sequences with rustyline invisible-character markers (`\x01`…`\x02`)
/// so rustyline measures prompt display width correctly (no cursor drift on long lines).
fn rl_escape(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 16);
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            result.push('\x01');
            result.push(c);
            for nc in chars.by_ref() {
                result.push(nc);
                if nc.is_ascii_alphabetic() {
                    result.push('\x02');
                    break;
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Build the shell prompt: `~/Workspace/orka (main) ❯ `
pub fn build_prompt(cwd: &Path, last_exit: Option<i32>) -> String {
    let dir = shorten_path(cwd);
    let branch = git_branch(cwd);
    let indicator = match last_exit {
        Some(0) | None => rl_escape(&"❯".green().to_string()),
        Some(_) => rl_escape(&"❯".red().to_string()),
    };
    if branch.is_empty() {
        format!("{dir} {indicator} ")
    } else {
        let branch_display = rl_escape(&branch.dimmed().to_string());
        format!("{dir} ({branch_display}) {indicator} ")
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
                std::path::PathBuf::from(gitdir).join("HEAD")
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
    fn rl_escape_wraps_ansi_sequences() {
        // A raw ANSI sequence: ESC[32m (green)
        let input = "\x1b[32mhello\x1b[0m";
        let escaped = rl_escape(input);
        // Each ESC[...m sequence should be wrapped with \x01 before and \x02 after
        assert!(escaped.contains("\x01\x1b[32m\x02"));
        assert!(escaped.contains("\x01\x1b[0m\x02"));
        assert!(escaped.contains("hello"));
    }

    #[test]
    fn rl_escape_plain_text_unchanged() {
        let input = "plain text";
        assert_eq!(rl_escape(input), input);
    }

    #[test]
    fn prompt_success_exit() {
        let p = build_prompt(Path::new("/tmp"), Some(0));
        assert!(p.contains("/tmp"));
        assert!(p.contains("❯"));
    }

    #[test]
    fn prompt_no_exit_yet() {
        let p = build_prompt(Path::new("/tmp"), None);
        assert!(p.contains("❯"));
    }
}
