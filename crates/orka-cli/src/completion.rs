use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use colored::Colorize;
use rustyline::completion::{Completer, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{Context, Helper};

/// Shell builtins handled by the `!` prefix.
const SHELL_BUILTINS: &[&str] = &["cd", "export", "unset", "history"];

/// Known local + server slash commands for completion.
const SLASH_COMMANDS: &[&str] = &[
    "/quit",
    "/exit",
    "/help",
    "/clear",
    "/skill",
    "/skills",
    "/reset",
    "/status",
    "/think",
    "/feedback",
    "/history",
    "/save",
];

/// Lazily populated list of executables from $PATH (populated on first tab-completion).
static PATH_COMMANDS: LazyLock<Vec<String>> = LazyLock::new(collect_path_commands);

/// Rustyline helper providing tab-completion for `!` shell commands,
/// `/` slash commands, and file paths.
pub struct OrkaHelper {
    /// Shared working directory, kept in sync with the REPL's `!cd` state.
    shell_cwd: Arc<Mutex<PathBuf>>,
    /// Colored version of the current prompt, updated by the REPL before each readline call.
    colored_prompt: Arc<Mutex<String>>,
}

impl OrkaHelper {
    pub fn new(shell_cwd: Arc<Mutex<PathBuf>>, colored_prompt: Arc<Mutex<String>>) -> Self {
        Self {
            shell_cwd,
            colored_prompt,
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
                let matches: Vec<Pair> = PATH_COMMANDS
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
            let dirs_only = parts[0].eq_ignore_ascii_case("cd");
            let cwd = self
                .shell_cwd
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let (start_in_rest, pairs) = complete_path(arg, dirs_only, &cwd);
            // Offset = 1 (for `!`) + command.len() + 1 (space) + start_in_rest
            let offset = if parts.len() == 2 {
                1 + parts[0].len() + 1 + start_in_rest
            } else {
                pos
            };
            return Ok((offset, pairs));
        }

        // After `/feedback ` — complete argument keywords
        if let Some(rest) = text.strip_prefix("/feedback ") {
            let prefix = rest.to_lowercase();
            let matches: Vec<Pair> = ["good", "bad"]
                .iter()
                .filter(|k| k.starts_with(prefix.as_str()))
                .map(|k| Pair {
                    display: k.to_string(),
                    replacement: k.to_string(),
                })
                .collect();
            return Ok((text.len() - rest.len(), matches));
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

        // After `@` — complete file paths for attachments.
        // Only trigger when `@` is at position 0 or preceded by whitespace
        // to avoid matching email addresses like user@example.com.
        if let Some(at_pos) = text.rfind('@') {
            let preceded_by_ws = at_pos == 0
                || text[..at_pos]
                    .chars()
                    .next_back()
                    .map(|c| c.is_whitespace())
                    .unwrap_or(true);
            if preceded_by_ws {
                let fragment = &text[at_pos + 1..];
                let cwd = self
                    .shell_cwd
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                let (start_in_fragment, pairs) = complete_path(fragment, false, &cwd);
                let offset = at_pos + 1 + start_in_fragment;
                return Ok((offset, pairs));
            }
        }

        Ok((pos, vec![]))
    }
}

impl Hinter for OrkaHelper {
    type Hint = String;

    fn hint(&self, line: &str, _pos: usize, _ctx: &Context<'_>) -> Option<String> {
        // Ghost text for unique `!` builtin prefix matches
        if let Some(rest) = line.strip_prefix('!') {
            if !rest.contains(' ') && !rest.is_empty() {
                let lower = rest.to_lowercase();
                let matches: Vec<&&str> = SHELL_BUILTINS
                    .iter()
                    .filter(|b| b.starts_with(lower.as_str()) && **b != lower.as_str())
                    .collect();
                if matches.len() == 1 {
                    return Some(matches[0][rest.len()..].to_string());
                }
            }
            return None;
        }

        // Ghost text for unique `/` command prefix matches
        if !line.starts_with('/') || line.contains(' ') {
            return None;
        }
        let lower = line.to_lowercase();
        let matches: Vec<&&str> = SLASH_COMMANDS
            .iter()
            .filter(|c| c.starts_with(lower.as_str()) && **c != lower.as_str())
            .collect();
        if matches.len() == 1 {
            Some(matches[0][line.len()..].to_string())
        } else {
            None
        }
    }
}

impl Highlighter for OrkaHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        if line.starts_with('!') {
            Cow::Owned(line.yellow().to_string())
        } else if line.starts_with('/') {
            Cow::Owned(line.cyan().to_string())
        } else {
            // Highlight `@<token>` in green for agent messages
            let highlighted = highlight_at_tokens(line);
            if highlighted == line {
                Cow::Borrowed(line)
            } else {
                Cow::Owned(highlighted)
            }
        }
    }

    fn highlight_char(
        &self,
        line: &str,
        _pos: usize,
        _forced: rustyline::highlight::CmdKind,
    ) -> bool {
        line.starts_with('!')
            || line.starts_with('/')
            || line.char_indices().any(|(i, c)| {
                c == '@'
                    && (i == 0
                        || line[..i]
                            .chars()
                            .next_back()
                            .map(|p| p.is_whitespace())
                            .unwrap_or(true))
            })
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        // UI-5: respect NO_COLOR — skip raw ANSI escape when the variable is set
        if std::env::var_os("NO_COLOR").is_some() {
            return Cow::Borrowed(hint);
        }
        // \x1b[90m = bright black (dark gray) — universally visible ghost text
        Cow::Owned(format!("\x1b[90m{hint}\x1b[0m"))
    }

    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        default: bool,
    ) -> Cow<'b, str> {
        if default {
            // Return the colored prompt stored by the REPL; the plain prompt was passed
            // to readline() so rustyline measures width correctly (no cursor drift).
            let guard = self
                .colored_prompt
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            Cow::Owned(guard.clone())
        } else {
            // Continuation prompt (multi-line)
            Cow::Owned(prompt.dimmed().to_string())
        }
    }
}

/// Core validation logic, extracted for testability (rustyline's `ValidationContext` is private).
fn validate_input(input: &str) -> ValidationResult {
    // Odd number of trailing backslashes → line continuation; even = escaped backslash.
    let trailing_backslashes = input.chars().rev().take_while(|&c| c == '\\').count();
    if trailing_backslashes % 2 == 1 {
        return ValidationResult::Incomplete;
    }

    // UI-14: count ``` only at the start of lines (after trim) to avoid
    // matching inline ``` in prose or being fooled by longer backtick runs.
    // An odd count means there's an open code fence → request continuation.
    let fence_count = input
        .lines()
        .filter(|line| line.trim_start().starts_with("```"))
        .count();
    if fence_count % 2 != 0 {
        return ValidationResult::Incomplete;
    }

    ValidationResult::Valid(None)
}

impl Validator for OrkaHelper {
    fn validate(&self, ctx: &mut ValidationContext<'_>) -> rustyline::Result<ValidationResult> {
        Ok(validate_input(ctx.input()))
    }

    fn validate_while_typing(&self) -> bool {
        false
    }
}

impl Helper for OrkaHelper {}

/// Highlight `@<token>` sequences in green.
/// Only triggers when `@` is at position 0 or preceded by whitespace
/// (same guard as `@` completion, to avoid matching email addresses).
fn highlight_at_tokens(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            let preceded_by_ws = i == 0 || chars[i - 1].is_whitespace();
            if preceded_by_ws {
                let token_start = i;
                i += 1;
                while i < chars.len() && !chars[i].is_whitespace() {
                    i += 1;
                }
                let token: String = chars[token_start..i].iter().collect();
                result.push_str(&token.green().to_string());
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

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
/// When `dirs_only` is true, only directory entries are included.
/// `base_dir` is used as the working directory when `fragment` has no `/` prefix.
fn complete_path(fragment: &str, dirs_only: bool, base_dir: &Path) -> (usize, Vec<Pair>) {
    let (dir, prefix) = if let Some(slash_pos) = fragment.rfind('/') {
        let dir_part = &fragment[..=slash_pos];
        let file_part = &fragment[slash_pos + 1..];
        (
            shellexpand_dir(dir_part),
            file_part.to_string(),
            // replacement starts after the last /
        )
    } else {
        (base_dir.to_path_buf(), fragment.to_string())
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
                let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                if dirs_only && !is_dir {
                    continue;
                }
                let suffix = if is_dir { "/" } else { " " };
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

    /// Mutex to serialise tests that mutate environment variables.
    static ENV_MUTEX: std::sync::LazyLock<std::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(()));

    fn test_helper() -> OrkaHelper {
        OrkaHelper::new(
            Arc::new(Mutex::new(PathBuf::from("."))),
            Arc::new(Mutex::new(String::new())),
        )
    }

    #[test]
    fn path_commands_not_empty() {
        let cmds = collect_path_commands();
        // On any unix system, there should be at least `ls`
        assert!(!cmds.is_empty());
    }

    #[test]
    fn complete_path_in_tmp() {
        // /tmp should exist and have entries on any Linux system
        let (offset, _pairs) = complete_path("/tmp/", false, Path::new("."));
        assert_eq!(offset, 5); // after "/tmp/"
    }

    #[test]
    fn slash_commands_complete() {
        let helper = test_helper();
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

    #[test]
    fn highlight_char_triggers_for_at_token() {
        let helper = test_helper();
        use rustyline::highlight::{CmdKind, Highlighter};
        assert!(helper.highlight_char("check @src/main.rs", 5, CmdKind::ForcedRefresh));
        assert!(helper.highlight_char("!ls", 1, CmdKind::ForcedRefresh));
        assert!(helper.highlight_char("/quit", 1, CmdKind::ForcedRefresh));
        assert!(!helper.highlight_char("plain text", 0, CmdKind::ForcedRefresh));
    }

    #[test]
    fn highlight_at_tokens_colors_at_paths() {
        let result = highlight_at_tokens("attach @src/main.rs here");
        assert!(result.contains("src/main.rs"));
        // Plain text not modified
        assert_eq!(highlight_at_tokens("hello world"), "hello world");
        // Email-like should not be highlighted (preceded by non-whitespace)
        let email = highlight_at_tokens("user@example.com");
        // The '@' is not at position 0 or after whitespace → no green wrapping
        assert!(!email.contains("\x1b[32m"));
    }

    #[test]
    fn feedback_completion() {
        let helper = test_helper();
        let hist = rustyline::history::DefaultHistory::new();
        let ctx = Context::new(&hist);
        let line = "/feedback g";
        let (start, matches) = helper.complete(line, line.len(), &ctx).unwrap();
        let names: Vec<&str> = matches.iter().map(|p| p.display.as_str()).collect();
        assert!(names.contains(&"good"));
        assert!(!names.contains(&"bad"));
        assert_eq!(start, "/feedback ".len());
    }

    #[test]
    fn builtin_hint_ghost_text() {
        let helper = test_helper();
        let hist = rustyline::history::DefaultHistory::new();
        let ctx = Context::new(&hist);
        // "!c" should hint "d" (completing "cd")
        let hint = helper.hint("!c", 2, &ctx);
        assert_eq!(hint.as_deref(), Some("d"));
        // "!exp" → "ort"
        let hint2 = helper.hint("!exp", 4, &ctx);
        assert_eq!(hint2.as_deref(), Some("ort"));
    }

    #[test]
    fn validator_trailing_backslash_incomplete() {
        assert!(matches!(
            validate_input("hello\\"),
            ValidationResult::Incomplete
        ));
    }

    #[test]
    fn validator_even_backslashes_valid() {
        assert!(matches!(
            validate_input("hello\\\\"),
            ValidationResult::Valid(_)
        ));
    }

    #[test]
    fn validator_odd_code_fences_incomplete() {
        assert!(matches!(
            validate_input("```rust\nfn main() {}"),
            ValidationResult::Incomplete
        ));
    }

    #[test]
    fn validator_even_code_fences_valid() {
        assert!(matches!(
            validate_input("```rust\nfn main() {}\n```"),
            ValidationResult::Valid(_)
        ));
    }

    #[test]
    fn highlight_hint_respects_no_color() {
        use rustyline::highlight::Highlighter;
        // Serialise env-var access across parallel test threads.
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let helper = test_helper();
        // SAFETY: guarded by ENV_MUTEX — no other test runs concurrently here.
        unsafe { std::env::set_var("NO_COLOR", "1") };
        let result = helper.highlight_hint("suggestion");
        unsafe { std::env::remove_var("NO_COLOR") };
        assert_eq!(&*result, "suggestion");
    }
}
