use std::path::Path;

use rustyline::completion::{Completer, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Context, Helper};

/// Known local + server slash commands for completion.
const SLASH_COMMANDS: &[&str] = &[
    "/quit", "/exit", "/help", "/clear", "/skill", "/skills", "/reset", "/status",
];

/// Rustyline helper providing tab-completion for `!` shell commands,
/// `/` slash commands, and file paths.
pub struct OrkaHelper {
    /// Cached sorted list of executables from $PATH.
    path_commands: Vec<String>,
}

impl OrkaHelper {
    pub fn new() -> Self {
        Self {
            path_commands: collect_path_commands(),
        }
    }
}

impl Completer for OrkaHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let text = &line[..pos];

        // After `!` — complete shell commands and file paths
        if let Some(rest) = text.strip_prefix('!') {
            // Skip builtins prefix (cd, export, etc.) — complete file paths for those
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() == 1 && !rest.ends_with(' ') {
                // Completing the command name itself
                let prefix = parts[0].to_lowercase();
                let matches: Vec<Pair> = self
                    .path_commands
                    .iter()
                    .filter(|c| c.starts_with(&prefix))
                    .map(|c| Pair {
                        display: c.clone(),
                        replacement: c.clone(),
                    })
                    .collect();
                // +1 to skip the `!`
                return Ok((1, matches));
            }
            // Completing arguments — try file path completion
            let arg = if parts.len() == 2 { parts[1] } else { "" };
            let (start_in_rest, pairs) = complete_path(arg);
            // Offset = 1 (for `!`) + command.len() + 1 (space) + start_in_rest
            let offset = if parts.len() == 2 {
                1 + parts[0].len() + 1 + start_in_rest
            } else {
                pos
            };
            return Ok((offset, pairs));
        }

        // After `/` — complete slash commands
        if text.starts_with('/') && !text.contains(' ') {
            let prefix = text.to_lowercase();
            let matches: Vec<Pair> = SLASH_COMMANDS
                .iter()
                .filter(|c| c.starts_with(&prefix))
                .map(|c| Pair {
                    display: c.to_string(),
                    replacement: c.to_string(),
                })
                .collect();
            return Ok((0, matches));
        }

        Ok((pos, vec![]))
    }
}

impl Hinter for OrkaHelper {
    type Hint = String;
}

impl Highlighter for OrkaHelper {}

impl Validator for OrkaHelper {}

impl Helper for OrkaHelper {}

/// Collect executable names from $PATH directories (deduplicated, sorted).
fn collect_path_commands() -> Vec<String> {
    let mut cmds = Vec::new();
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(':') {
            let dir = Path::new(dir);
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if let Ok(ft) = entry.file_type()
                        && (ft.is_file() || ft.is_symlink())
                        && let Some(name) = entry.file_name().to_str()
                    {
                        cmds.push(name.to_string());
                    }
                }
            }
        }
    }
    cmds.sort_unstable();
    cmds.dedup();
    cmds
}

/// Complete a file path fragment. Returns (replacement_start_offset, pairs).
fn complete_path(fragment: &str) -> (usize, Vec<Pair>) {
    let (dir, prefix) = if let Some(slash_pos) = fragment.rfind('/') {
        let dir_part = &fragment[..=slash_pos];
        let file_part = &fragment[slash_pos + 1..];
        (
            shellexpand_dir(dir_part),
            file_part.to_string(),
            // replacement starts after the last /
        )
    } else {
        (
            std::env::current_dir().unwrap_or_default(),
            fragment.to_string(),
        )
    };

    let start_offset = if fragment.contains('/') {
        fragment.rfind('/').unwrap() + 1
    } else {
        0
    };

    let mut pairs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str()
                && name.starts_with(&prefix)
            {
                let suffix = if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                    "/"
                } else {
                    " "
                };
                pairs.push(Pair {
                    display: name.to_string(),
                    replacement: format!("{name}{suffix}"),
                });
            }
        }
    }
    pairs.sort_by(|a, b| a.display.cmp(&b.display));
    (start_offset, pairs)
}

fn shellexpand_dir(s: &str) -> std::path::PathBuf {
    if let Some(rest) = s.strip_prefix('~')
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest.strip_prefix('/').unwrap_or(rest));
    }
    std::path::PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_commands_not_empty() {
        let cmds = collect_path_commands();
        // On any unix system, there should be at least `ls`
        assert!(!cmds.is_empty());
    }

    #[test]
    fn complete_path_in_tmp() {
        // /tmp should exist and have entries on any Linux system
        let (offset, _pairs) = complete_path("/tmp/");
        assert_eq!(offset, 5); // after "/tmp/"
    }

    #[test]
    fn slash_commands_complete() {
        let helper = OrkaHelper::new();
        let (start, matches) = helper
            .complete(
                "/sk",
                3,
                &Context::new(&rustyline::history::DefaultHistory::new()),
            )
            .unwrap();
        assert_eq!(start, 0);
        let names: Vec<&str> = matches.iter().map(|p| p.display.as_str()).collect();
        assert!(names.contains(&"/skill"));
        assert!(names.contains(&"/skills"));
    }
}
